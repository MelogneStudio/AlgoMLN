use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::broker::Timeframe;
use crate::models::{Candle, Position};
use crate::plugin::api::events::{EventBus, EventKind};
use crate::strategy::dsl::{
    ActionNode, CompareOp, ConditionNode, ExprNode, IndicatorKind, PriceField, RuleNode,
    StrategyNode,
};
use crate::strategy::execution::{
    build_order, ExecutionTarget, OrderBuildError, PaperPosition, PaperTrade,
};
use crate::strategy::logging::{LogEntry, LogEntryKind, RuleSkipReason, StrategyLogger};

use super::context::EvalContext;
use super::cross::CrossDetector;
use super::incremental_provider::BoundedWindowProvider;
use super::indicator_provider::{IndicatorProvider, IndicatorProviderProfile};
use super::trigger_state::TriggerStateMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StrategyStatus {
    Running,
    Paused,
    Stopped,
    Error(String),
}

pub struct StrategyInstance {
    pub id: String,
    pub strategy: Arc<StrategyNode>,
    pub symbol: String,
    pub timeframe: Timeframe,
    pub status: StrategyStatus,
    pub execution_target: Arc<dyn ExecutionTarget>,
}

#[derive(Debug, Clone, Serialize)]
pub enum EvalError {
    InsufficientData {
        indicator: IndicatorKind,
        period: usize,
        available: usize,
    },
    InsufficientHistory {
        required: usize,
        available: usize,
    },
    NotYetImplemented(&'static str),
    OrderBuildFailed(String),
    EmptyCandles,
}

pub struct StrategyEngine {
    pub instance: StrategyInstance,
    cross_detector: CrossDetector,
    trigger_state: TriggerStateMap,
    indicator_provider: Box<dyn IndicatorProvider>,
    logger: StrategyLogger,
    profile: StrategyEngineProfile,
    /// Optional event bus used to broadcast engine events to plugins. Set to
    /// `None` for backtest paths (determinism) and live/paper paths until the
    /// stage-9 wiring lands. See `src-tauri/src/main.rs` for the live hook.
    pub event_bus: Option<Arc<EventBus>>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct StrategyEngineProfile {
    pub on_candle_calls: u64,
    pub on_candle_time: Duration,
    pub broker_execute_calls: u64,
    pub broker_execute_time: Duration,
    pub broker_get_positions_calls: u64,
    pub broker_get_positions_time: Duration,
}

impl StrategyEngine {
    pub fn new(instance: StrategyInstance) -> Self {
        let logger = StrategyLogger::new(instance.id.clone());
        Self {
            instance,
            cross_detector: CrossDetector::new(),
            trigger_state: TriggerStateMap::new(),
            indicator_provider: Box::new(BoundedWindowProvider::new()),
            logger,
            profile: StrategyEngineProfile::default(),
            event_bus: None,
        }
    }

    pub async fn on_candle(&mut self, candles: &[Candle]) -> Vec<LogEntry> {
        let started = Instant::now();
        self.profile.on_candle_calls += 1;

        if !matches!(self.instance.status, StrategyStatus::Running) {
            self.profile.on_candle_time += started.elapsed();
            return Vec::new();
        }

        let Some(ctx) = EvalContext::new(candles) else {
            self.profile.on_candle_time += started.elapsed();
            return Vec::new();
        };

        self.indicator_provider.clear_cache();
        let rules = self.instance.strategy.rules.clone();

        for rule in &rules {
            let prev_state = self.trigger_state.was_true(&rule.id);
            match self.evaluate_rule(rule, &ctx) {
                Ok(Some(action)) => {
                    self.logger.log(
                        LogEntryKind::ConditionEvaluated {
                            rule_id: rule.id.clone(),
                            result: true,
                            prev_state,
                            fired: true,
                            indicator_snapshots: Vec::new(),
                        },
                        ctx.current.timestamp,
                    );
                    self.logger.log(
                        LogEntryKind::RuleFired {
                            rule_id: rule.id.clone(),
                            action: action.clone(),
                        },
                        ctx.current.timestamp,
                    );
                    if let Some(bus) = &self.event_bus {
                        bus.publish(EventKind::RuleFired {
                            rule_id: rule.id.clone(),
                            strategy_id: self.instance.id.clone(),
                        });
                    }
                    self.submit_action(rule, action, &ctx).await;
                }
                Ok(None) => {
                    self.logger.log(
                        LogEntryKind::ConditionEvaluated {
                            rule_id: rule.id.clone(),
                            result: self.trigger_state.was_true(&rule.id),
                            prev_state,
                            fired: false,
                            indicator_snapshots: Vec::new(),
                        },
                        ctx.current.timestamp,
                    );
                }
                Err(error) => {
                    self.trigger_state.should_fire(&rule.id, false);
                    self.logger.log(
                        LogEntryKind::EvalError {
                            rule_id: rule.id.clone(),
                            error: format!("{error:?}"),
                        },
                        ctx.current.timestamp,
                    );
                }
            }
        }

        for rule in &rules {
            self.update_cross_state(rule, &ctx);
        }

        if let Some(bus) = &self.event_bus {
            bus.publish(EventKind::CandleProcessed(ctx.current.clone()));
        }

        self.indicator_provider.advance(ctx.current);
        let entries = self.logger.drain_entries();
        self.profile.on_candle_time += started.elapsed();
        entries
    }

    pub fn profile(&self) -> StrategyEngineProfile {
        self.profile
    }

    pub fn indicator_profile(&self) -> IndicatorProviderProfile {
        self.indicator_provider.profile()
    }

    fn evaluate_rule(
        &mut self,
        rule: &RuleNode,
        ctx: &EvalContext<'_>,
    ) -> Result<Option<ActionNode>, EvalError> {
        let condition_result = eval_condition(
            &rule.condition,
            ctx,
            self.indicator_provider.as_mut(),
            &self.cross_detector,
            &rule.id,
        )?;
        let should_fire = self.trigger_state.should_fire(&rule.id, condition_result);
        Ok(should_fire.then(|| rule.action.clone()))
    }

    async fn submit_action(&mut self, rule: &RuleNode, action: ActionNode, ctx: &EvalContext<'_>) {
        let current_position = self.current_paper_position().await;
        let order = match build_order(
            &action,
            &self.instance.symbol,
            ctx.current.close,
            current_position.as_ref(),
            &rule.id,
        ) {
            Ok(order) => order,
            Err(error) => {
                match error {
                    OrderBuildError::NoPosition => self.logger.log(
                        LogEntryKind::RuleSkipped {
                            rule_id: rule.id.clone(),
                            reason: RuleSkipReason::NoPosition,
                        },
                        ctx.current.timestamp,
                    ),
                    OrderBuildError::ZeroQuantity => self.logger.log(
                        LogEntryKind::OrderFailed {
                            rule_id: rule.id.clone(),
                            error: "zero quantity - validator missed this".to_string(),
                        },
                        ctx.current.timestamp,
                    ),
                    OrderBuildError::QuantityTooLarge => self.logger.log(
                        LogEntryKind::OrderFailed {
                            rule_id: rule.id.clone(),
                            error: "quantity too large".to_string(),
                        },
                        ctx.current.timestamp,
                    ),
                }
                return;
            }
        };

        self.logger.log(
            LogEntryKind::OrderSubmitted {
                rule_id: rule.id.clone(),
                order: order.clone(),
            },
            ctx.current.timestamp,
        );

        let started = Instant::now();
        self.profile.broker_execute_calls += 1;
        let execution_result = self.instance.execution_target.execute(order).await;
        self.profile.broker_execute_time += started.elapsed();

        match execution_result {
            Ok(result) => {
                self.logger.log(
                    LogEntryKind::OrderExecuted {
                        rule_id: rule.id.clone(),
                        result: result.clone(),
                    },
                    ctx.current.timestamp,
                );
                if let Some(bus) = &self.event_bus {
                    if let Some(trade) = self.latest_paper_trade() {
                        bus.publish(EventKind::TradeExecuted(trade));
                    }
                }
            }
            Err(error) => self.logger.log(
                LogEntryKind::OrderFailed {
                    rule_id: rule.id.clone(),
                    error: error.message,
                },
                ctx.current.timestamp,
            ),
        }
    }

    async fn current_paper_position(&mut self) -> Option<PaperPosition> {
        let started = Instant::now();
        self.profile.broker_get_positions_calls += 1;
        let positions = self.instance.execution_target.get_positions().await.ok()?;
        self.profile.broker_get_positions_time += started.elapsed();
        positions
            .into_iter()
            .find(|position| position.symbol == self.instance.symbol)
            .map(position_to_paper)
    }

    /// Pull the most recent paper trade from the execution target, if it's a
    /// `PaperBroker`. Returns `None` for live brokers or if no trade has been
    /// recorded yet. Used to fire `EventKind::TradeExecuted` after a successful
    /// execution.
    fn latest_paper_trade(&self) -> Option<PaperTrade> {
        if !self.instance.execution_target.is_paper() {
            return None;
        }
        let any = self.instance.execution_target.as_any();
        let paper = any.downcast_ref::<crate::strategy::execution::PaperBroker>()?;
        paper.get_state().trade_history.last().cloned()
    }

    fn update_cross_state(&mut self, rule: &RuleNode, ctx: &EvalContext<'_>) {
        for (rule_id, fast, slow) in collect_cross_values(
            &rule.id,
            &rule.condition,
            ctx,
            self.indicator_provider.as_mut(),
        ) {
            self.cross_detector.update(&rule_id, fast, slow);
        }
    }
}

fn eval_condition(
    condition: &ConditionNode,
    ctx: &EvalContext<'_>,
    provider: &mut dyn IndicatorProvider,
    cross_detector: &CrossDetector,
    rule_id: &str,
) -> Result<bool, EvalError> {
    match condition {
        ConditionNode::Comparison { left, op, right } => {
            let left = eval_expr(left, ctx, provider)?;
            let right = eval_expr(right, ctx, provider)?;
            Ok(compare(left, op, right))
        }
        ConditionNode::CrossAbove { fast, slow } => {
            let fast = eval_expr(fast, ctx, provider)?;
            let slow = eval_expr(slow, ctx, provider)?;
            Ok(cross_detector.is_cross_above(rule_id, fast, slow))
        }
        ConditionNode::CrossBelow { fast, slow } => {
            let fast = eval_expr(fast, ctx, provider)?;
            let slow = eval_expr(slow, ctx, provider)?;
            Ok(cross_detector.is_cross_below(rule_id, fast, slow))
        }
        ConditionNode::And(left, right) => {
            if !eval_condition(left, ctx, provider, cross_detector, rule_id)? {
                return Ok(false);
            }
            eval_condition(right, ctx, provider, cross_detector, rule_id)
        }
        ConditionNode::Or(left, right) => {
            if eval_condition(left, ctx, provider, cross_detector, rule_id)? {
                return Ok(true);
            }
            eval_condition(right, ctx, provider, cross_detector, rule_id)
        }
        ConditionNode::Not(inner) => Ok(!eval_condition(
            inner,
            ctx,
            provider,
            cross_detector,
            rule_id,
        )?),
        ConditionNode::InPosition => Err(EvalError::NotYetImplemented("in_position")),
        ConditionNode::TimeWindow { .. } => Err(EvalError::NotYetImplemented("between")),
    }
}

fn eval_expr(
    expr: &ExprNode,
    ctx: &EvalContext<'_>,
    provider: &mut dyn IndicatorProvider,
) -> Result<f64, EvalError> {
    match expr {
        ExprNode::Literal(value) => Ok(*value),
        ExprNode::PriceField(field) => Ok(match field {
            PriceField::Close => ctx.current.close,
            PriceField::Open => ctx.current.open,
            PriceField::High => ctx.current.high,
            PriceField::Low => ctx.current.low,
            PriceField::Volume => ctx.current.volume,
            PriceField::PrevClose => {
                if ctx.candles.len() < 2 {
                    return Err(EvalError::InsufficientHistory {
                        required: 2,
                        available: ctx.candles.len(),
                    });
                }
                ctx.candles[ctx.candles.len() - 2].close
            }
            PriceField::PrevOpen => {
                if ctx.candles.len() < 2 {
                    return Err(EvalError::InsufficientHistory {
                        required: 2,
                        available: ctx.candles.len(),
                    });
                }
                ctx.candles[ctx.candles.len() - 2].open
            }
            PriceField::PrevHigh => {
                if ctx.candles.len() < 2 {
                    return Err(EvalError::InsufficientHistory {
                        required: 2,
                        available: ctx.candles.len(),
                    });
                }
                ctx.candles[ctx.candles.len() - 2].high
            }
            PriceField::PrevLow => {
                if ctx.candles.len() < 2 {
                    return Err(EvalError::InsufficientHistory {
                        required: 2,
                        available: ctx.candles.len(),
                    });
                }
                ctx.candles[ctx.candles.len() - 2].low
            }
        }),
        ExprNode::Indicator(call) => {
            provider
                .get(&call.kind, call.period, ctx.candles)
                .ok_or(EvalError::InsufficientData {
                    indicator: call.kind.clone(),
                    period: call.period,
                    available: ctx.candles.len(),
                })
        }
    }
}

fn compare(left: f64, op: &CompareOp, right: f64) -> bool {
    match op {
        CompareOp::Lt => left < right,
        CompareOp::Gt => left > right,
        CompareOp::Lte => left <= right,
        CompareOp::Gte => left >= right,
        CompareOp::Eq => (left - right).abs() <= f64::EPSILON,
        CompareOp::Neq => (left - right).abs() > f64::EPSILON,
    }
}

fn collect_cross_values(
    rule_id: &str,
    condition: &ConditionNode,
    ctx: &EvalContext<'_>,
    provider: &mut dyn IndicatorProvider,
) -> Vec<(String, f64, f64)> {
    let mut values = Vec::new();
    match condition {
        ConditionNode::CrossAbove { fast, slow } | ConditionNode::CrossBelow { fast, slow } => {
            if let (Ok(fast), Ok(slow)) = (
                eval_expr(fast, ctx, provider),
                eval_expr(slow, ctx, provider),
            ) {
                values.push((rule_id.to_string(), fast, slow));
            }
        }
        ConditionNode::And(left, right) | ConditionNode::Or(left, right) => {
            values.extend(collect_cross_values(rule_id, left, ctx, provider));
            values.extend(collect_cross_values(rule_id, right, ctx, provider));
        }
        ConditionNode::Not(inner) => {
            values.extend(collect_cross_values(rule_id, inner, ctx, provider));
        }
        _ => {}
    }
    values
}

fn position_to_paper(position: Position) -> PaperPosition {
    PaperPosition {
        symbol: position.symbol,
        quantity: position.quantity,
        avg_entry_price: position.average_price,
        unrealized_pnl: position.unrealized_pnl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::dsl::{AstValidator, Lexer, Parser};
    use crate::strategy::execution::PaperBroker;

    fn candle(close: f64) -> Candle {
        Candle {
            timestamp: close as i64,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1000.0,
        }
    }

    fn make_engine(source: &str, initial_cash: f64) -> StrategyEngine {
        let tokens = Lexer::tokenize(source).unwrap();
        let node = Parser::new(tokens).parse().unwrap();
        let errors = AstValidator::validate(&node);
        assert!(errors.is_empty(), "validation failed: {errors:?}");
        let broker = Arc::new(PaperBroker::new("TEST".to_string(), initial_cash));
        let instance = StrategyInstance {
            id: "test-strategy".to_string(),
            strategy: Arc::new(node),
            symbol: "TEST".to_string(),
            timeframe: Timeframe::M5,
            status: StrategyStatus::Running,
            execution_target: broker,
        };
        StrategyEngine::new(instance)
    }

    #[tokio::test]
    async fn fires_exactly_once_on_condition_trigger() {
        let mut engine = make_engine("WHEN close > 105\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = (100..=108).map(|close| candle(close as f64)).collect();
        let mut total_trades = 0;

        for index in 1..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_trades += logs
                .iter()
                .filter(|entry| matches!(entry.kind, LogEntryKind::OrderExecuted { .. }))
                .count();
        }

        assert_eq!(total_trades, 1);
    }

    #[tokio::test]
    async fn idiot_test_fires_only_once() {
        let mut engine = make_engine("WHEN close > 0\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = (1..=20).map(|close| candle(close as f64)).collect();
        let mut total_trades = 0;

        for index in 1..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_trades += logs
                .iter()
                .filter(|entry| matches!(entry.kind, LogEntryKind::OrderExecuted { .. }))
                .count();
        }

        assert_eq!(total_trades, 1);
    }

    #[tokio::test]
    async fn fires_again_after_condition_resets() {
        let mut engine = make_engine("WHEN close > 105\nBUY 1", 100_000.0);
        let candles = vec![candle(100.0), candle(106.0), candle(100.0), candle(106.0)];
        let mut total_trades = 0;

        for index in 1..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_trades += logs
                .iter()
                .filter(|entry| matches!(entry.kind, LogEntryKind::OrderExecuted { .. }))
                .count();
        }

        assert_eq!(total_trades, 2);
    }

    #[tokio::test]
    async fn cross_above_fires_exactly_once() {
        let mut engine = make_engine("WHEN cross_above(ma(2), ma(5))\nBUY 1", 100_000.0);
        let closes = [50.0, 49.0, 48.0, 47.0, 46.0, 45.0, 90.0, 92.0, 93.0, 94.0];
        let candles: Vec<Candle> = closes.iter().copied().map(candle).collect();
        let mut total_trades = 0;

        for index in 1..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_trades += logs
                .iter()
                .filter(|entry| matches!(entry.kind, LogEntryKind::OrderExecuted { .. }))
                .count();
        }

        assert_eq!(total_trades, 1);
    }

    #[tokio::test]
    async fn engine_skips_evaluation_when_paused() {
        let mut engine = make_engine("WHEN close > 0\nBUY 1", 100_000.0);
        engine.instance.status = StrategyStatus::Paused;
        let candles: Vec<Candle> = (1..=5).map(|close| candle(close as f64)).collect();

        let logs = engine.on_candle(&candles).await;

        assert!(logs.is_empty());
    }

    #[tokio::test]
    async fn prev_close_requires_previous_candle() {
        let mut engine = make_engine("WHEN close > prev_close\nBUY 1", 100_000.0);
        let candles = vec![candle(100.0)];

        let logs = engine.on_candle(&candles).await;

        assert!(logs.iter().any(|entry| {
            matches!(
                &entry.kind,
                LogEntryKind::EvalError {
                    error,
                    ..
                } if error.contains("InsufficientHistory")
            )
        }));
    }
}
