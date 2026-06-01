use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::broker::Timeframe;
use crate::models::{Candle, Position};
use crate::strategy::dsl::{
    ActionNode, CompareOp, ConditionNode, ExprNode, IndicatorKind, PriceField, RuleNode,
    StrategyNode,
};
use crate::strategy::execution::{build_order, ExecutionTarget, PaperPosition};
use crate::strategy::logging::{LogEntry, LogEntryKind, StrategyLogger};

use super::context::EvalContext;
use super::cross::CrossDetector;
use super::indicator_provider::{FullRecomputeProvider, IndicatorProvider};
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
}

impl StrategyEngine {
    pub fn new(instance: StrategyInstance) -> Self {
        let logger = StrategyLogger::new(instance.id.clone());
        Self {
            instance,
            cross_detector: CrossDetector::new(),
            trigger_state: TriggerStateMap::new(),
            indicator_provider: Box::new(FullRecomputeProvider::new()),
            logger,
        }
    }

    pub async fn on_candle(&mut self, candles: &[Candle]) -> Vec<LogEntry> {
        if !matches!(self.instance.status, StrategyStatus::Running) {
            return Vec::new();
        }

        let Some(ctx) = EvalContext::new(candles) else {
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

        self.indicator_provider.advance(ctx.current);
        self.logger.drain_entries()
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
                self.logger.log(
                    LogEntryKind::OrderFailed {
                        rule_id: rule.id.clone(),
                        error: format!("{error:?}"),
                    },
                    ctx.current.timestamp,
                );
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

        match self.instance.execution_target.execute(order).await {
            Ok(result) => self.logger.log(
                LogEntryKind::OrderExecuted {
                    rule_id: rule.id.clone(),
                    result,
                },
                ctx.current.timestamp,
            ),
            Err(error) => self.logger.log(
                LogEntryKind::OrderFailed {
                    rule_id: rule.id.clone(),
                    error: error.message,
                },
                ctx.current.timestamp,
            ),
        }
    }

    async fn current_paper_position(&self) -> Option<PaperPosition> {
        let positions = self.instance.execution_target.get_positions().await.ok()?;
        positions
            .into_iter()
            .find(|position| position.symbol == self.instance.symbol)
            .map(position_to_paper)
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
}
