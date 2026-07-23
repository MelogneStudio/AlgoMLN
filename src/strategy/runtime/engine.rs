use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::broker::Timeframe;
use crate::models::{Candle, Position};
use crate::plugin::api::events::{EventBus, EventKind};
use crate::strategy::dsl::{
    ActionNode, CompareOp, ConditionNode, ExprNode, IndicatorKind, PriceField, RiskConfig, RuleNode,
    StrategyNode,
};
use crate::strategy::execution::{
    build_order, ExecutionTarget, OrderBuildError, PaperPosition, PaperTrade,
};
use crate::strategy::logging::{
    LogEntry, LogEntryKind, RiskBreachReason, RuleSkipReason, StrategyLogger,
};

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
    /// Per-run risk-control state. Initialized from
    /// `instance.strategy.risk`; only allocated when at least one risk
    /// declaration is present (so strategies without `RISK` pay nothing).
    risk_state: Option<RiskState>,
}

/// Tracks the limits declared via `RISK MAX_ORDERS` and the cumulative
/// realized loss. `daily_realized_loss` is session-scoped (in a backtest
/// "session" = the whole run; in a live paper run = the lifetime of the
/// strategy instance).
#[derive(Debug, Clone)]
struct RiskState {
    session_orders: u32,
    daily_realized_loss: f64,
}

impl RiskState {
    fn new() -> Self {
        Self {
            session_orders: 0,
            daily_realized_loss: 0.0,
        }
    }
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
        let risk_state = if instance.strategy.risk.is_some() {
            Some(RiskState::new())
        } else {
            None
        };
        Self {
            instance,
            cross_detector: CrossDetector::new(),
            trigger_state: TriggerStateMap::new(),
            indicator_provider: Box::new(BoundedWindowProvider::new()),
            logger,
            profile: StrategyEngineProfile::default(),
            event_bus: None,
            risk_state,
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
                    self.submit_action(&rule.id, action, &ctx).await;
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

        // After the rule loop and the cross-state update pass, run the
        // stop-loss / take-profit pass on the current open position. This
        // deliberately runs after the rule loop so a rule that fires on the
        // same candle (closing or opening a position) is reflected in the
        // position we evaluate here. It also runs after the cross-update
        // pass for symmetry with the other post-rule bookkeeping.
        if self.instance.strategy.stop_loss.is_some() || self.instance.strategy.take_profit.is_some() {
            self.run_stop_loss_take_profit_pass(&ctx).await;
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

    async fn submit_action(
        &mut self,
        source_id: &str,
        action: ActionNode,
        ctx: &EvalContext<'_>,
    ) {
        // Run risk-control checks before building the order. If any limit is
        // breached we log a `RiskBreach` entry and return without
        // touching the broker. Order is evaluated after the position
        // snapshot below — we need positions to count open ones for
        // MAX_POSITIONS, and the realized-loss number for MAX_DAILY_LOSS.
        if let Some(breach) = self.check_risk_breach(&action, ctx).await {
            self.logger.log(
                LogEntryKind::RiskBreach {
                    rule_id: source_id.to_string(),
                    reason: breach,
                },
                ctx.current.timestamp,
            );
            return;
        }

        let current_position = self.current_paper_position().await;
        let available_cash = self.instance.execution_target.available_cash();
        let order = match build_order(
            &action,
            &self.instance.symbol,
            ctx.current.close,
            available_cash,
            current_position.as_ref(),
            source_id,
        ) {
            Ok(order) => order,
            Err(error) => {
                match error {
                    OrderBuildError::NoPosition => self.logger.log(
                        LogEntryKind::RuleSkipped {
                            rule_id: source_id.to_string(),
                            reason: RuleSkipReason::NoPosition,
                        },
                        ctx.current.timestamp,
                    ),
                    OrderBuildError::InsufficientCash => self.logger.log(
                        LogEntryKind::RuleSkipped {
                            rule_id: source_id.to_string(),
                            reason: RuleSkipReason::InsufficientCash,
                        },
                        ctx.current.timestamp,
                    ),
                    OrderBuildError::ZeroQuantity => self.logger.log(
                        LogEntryKind::OrderFailed {
                            rule_id: source_id.to_string(),
                            error: "zero quantity - validator missed this".to_string(),
                        },
                        ctx.current.timestamp,
                    ),
                    OrderBuildError::QuantityTooLarge => self.logger.log(
                        LogEntryKind::OrderFailed {
                            rule_id: source_id.to_string(),
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
                rule_id: source_id.to_string(),
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
                // Increment the session order counter only on a
                // successful submit — failed orders (insufficient funds,
                // insufficient position) do not count toward MAX_ORDERS.
                if let Some(state) = self.risk_state.as_mut() {
                    state.session_orders += 1;
                }
                self.logger.log(
                    LogEntryKind::OrderExecuted {
                        rule_id: source_id.to_string(),
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
                    rule_id: source_id.to_string(),
                    error: error.message,
                },
                ctx.current.timestamp,
            ),
        }
    }

    /// Check the strategy's `RISK` declarations against current state and
    /// return the first `RiskBreachReason` that fires, or `None` if the
    /// order may proceed. Order:
    ///   1. `MAX_ORDERS` — count is in `risk_state.session_orders`, so no
    ///      broker call is needed.
    ///   2. `MAX_POSITIONS` — counts open positions (`quantity > 0`) on the
    ///      broker. Applies only to BUY actions; sells are never blocked by
    ///      this check.
    ///   3. `MAX_DAILY_LOSS` — uses `execution_target.realized_loss()` and
    ///      `available_cash()` to compute the cumulative loss as a
    ///      percentage of `initial_cash`. In a backtest there's no clock, so
    ///      "daily" is session-scoped (cumulative). When breached, all
    ///      subsequent orders — both buys and sells — are blocked.
    async fn check_risk_breach(
        &mut self,
        action: &ActionNode,
        _ctx: &EvalContext<'_>,
    ) -> Option<RiskBreachReason> {
        let risk: &RiskConfig = self.instance.strategy.risk.as_ref()?;

        if let Some(limit) = risk.max_orders {
            let session_orders = self
                .risk_state
                .as_ref()
                .expect("risk_state must be Some when strategy.risk is Some")
                .session_orders;
            if session_orders >= limit {
                return Some(RiskBreachReason::MaxOrdersReached);
            }
        }

        if matches!(action, ActionNode::Buy { .. }) {
            if let Some(limit) = risk.max_open_positions {
                let started = std::time::Instant::now();
                self.profile.broker_get_positions_calls += 1;
                let positions = self
                    .instance
                    .execution_target
                    .get_positions()
                    .await
                    .ok()
                    .unwrap_or_default();
                self.profile.broker_get_positions_time += started.elapsed();
                let open_count = positions
                    .iter()
                    .filter(|position| position.quantity > 0)
                    .count() as u32;
                if open_count >= limit {
                    return Some(RiskBreachReason::MaxPositionsReached);
                }
            }
        }

        if let Some(limit_pct) = risk.max_daily_loss_pct {
            let initial = self.broker_initial_cash();
            if initial > 0.0 {
                let realized = self.instance.execution_target.realized_loss();
                let loss_pct = realized / initial * 100.0;
                if loss_pct >= limit_pct {
                    // Refresh the cached session loss so subsequent
                    // breaches within the same candle don't redundantly
                    // recompute from the broker.
                    if let Some(state) = self.risk_state.as_mut() {
                        state.daily_realized_loss = realized;
                    }
                    return Some(RiskBreachReason::MaxDailyLossReached);
                }
                if let Some(state) = self.risk_state.as_mut() {
                    state.daily_realized_loss = realized;
                }
            }
        }

        None
    }

    /// Read `initial_cash` from the broker's public state, if it's a
    /// `PaperBroker`. Returns 0.0 for any other `ExecutionTarget` so the
    /// loss-percentage check degrades to "never breached" rather than
    /// dividing by zero or panicking.
    fn broker_initial_cash(&self) -> f64 {
        let any = self.instance.execution_target.as_any();
        if let Some(paper) =
            any.downcast_ref::<crate::strategy::execution::PaperBroker>()
        {
            paper.get_state().initial_cash
        } else {
            0.0
        }
    }

    /// Run the strategy-level stop-loss / take-profit pass on the current
    /// candle. For each open position (`quantity > 0`) on the engine's
    /// symbol, compute the unrealized loss/gain against the entry price and,
    /// if either threshold is breached, submit a `SELL ALL` order through the
    /// normal order path. If both thresholds would fire on the same candle
    /// (e.g. a gap candle), stop-loss fires first and take-profit is
    /// skipped — the position is already closed by the SL pass.
    ///
    /// This deliberately bypasses `TriggerStateMap` and fires every candle
    /// while the position is underwater or in profit; it is the strategy's
    /// safety net, not an edge-triggered rule.
    async fn run_stop_loss_take_profit_pass(&mut self, ctx: &EvalContext<'_>) {
        let stop_loss = self.instance.strategy.stop_loss;
        let take_profit = self.instance.strategy.take_profit;

        // Snapshot the open position once. After the SL pass, the position
        // may be closed — TP must not refire against a position that no
        // longer exists.
        let position = match self.current_paper_position().await {
            Some(pos) if pos.quantity > 0 => pos,
            _ => return,
        };

        // Sanity check: the position is in this engine's symbol. In
        // single-symbol backtests / paper runs this is always true; the
        // PortfolioEngine routes per-symbol so each sub-engine sees only
        // its own position.
        if position.symbol != self.instance.symbol {
            return;
        }

        // Need a positive entry price to compute a percentage.
        if position.avg_entry_price <= 0.0 {
            return;
        }

        let current_close = ctx.current.close;
        if current_close <= 0.0 {
            return;
        }

        let entry = position.avg_entry_price;
        let loss_pct = (entry - current_close) / entry * 100.0;
        let gain_pct = (current_close - entry) / entry * 100.0;

        // Stop loss first — if it fires, the position closes and take
        // profit is skipped.
        if let Some(threshold) = stop_loss {
            if loss_pct >= threshold {
                self.logger.log(
                    LogEntryKind::StopLossFired {
                        symbol: self.instance.symbol.clone(),
                        loss_pct,
                        price: current_close,
                    },
                    ctx.current.timestamp,
                );
                self.submit_action("stop_loss", ActionNode::SellAll, ctx)
                    .await;
                return;
            }
        }

        if let Some(threshold) = take_profit {
            if gain_pct >= threshold {
                self.logger.log(
                    LogEntryKind::TakeProfitFired {
                        symbol: self.instance.symbol.clone(),
                        gain_pct,
                        price: current_close,
                    },
                    ctx.current.timestamp,
                );
                self.submit_action("take_profit", ActionNode::SellAll, ctx)
                    .await;
            }
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

    // ---------- STOP_LOSS / TAKE_PROFIT ----------

    #[tokio::test]
    async fn stop_loss_fires_when_loss_breaches_threshold() {
        // Stop loss 2%: open at 100, drop to 97 → 3% loss → SL fires.
        let mut engine = make_engine("STOP_LOSS 2%\nWHEN close > 0\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = vec![candle(100.0), candle(100.0), candle(97.0)];

        // Candle 1: BUY fires (close > 0, fresh trigger).
        let logs1 = engine.on_candle(&candles[..1]).await;
        assert!(logs1
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. })));

        // Candle 2: no rule fire (trigger state held), no SL (not underwater).
        let logs2 = engine.on_candle(&candles[..2]).await;
        assert!(!logs2
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::StopLossFired { .. })));

        // Candle 3: 3% loss > 2% threshold → SL fires, position closes.
        let logs3 = engine.on_candle(&candles[..3]).await;
        assert!(logs3
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::StopLossFired { .. })));
        assert!(logs3
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. })));

        // Position should now be closed.
        let state = engine
            .instance
            .execution_target
            .as_any()
            .downcast_ref::<crate::strategy::execution::PaperBroker>()
            .unwrap()
            .get_state();
        assert!(state.positions.is_empty());
    }

    #[tokio::test]
    async fn stop_loss_does_not_fire_when_loss_under_threshold() {
        // Stop loss 5%: open at 100, drop to 97 → 3% loss < 5% threshold.
        let mut engine = make_engine("STOP_LOSS 5%\nWHEN close > 0\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = vec![candle(100.0), candle(100.0), candle(97.0)];

        engine.on_candle(&candles[..1]).await;
        engine.on_candle(&candles[..2]).await;
        let logs3 = engine.on_candle(&candles[..3]).await;

        assert!(!logs3
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::StopLossFired { .. })));
    }

    #[tokio::test]
    async fn take_profit_fires_when_gain_breaches_threshold() {
        // Take profit 5%: open at 100, rise to 106 → 6% gain → TP fires.
        let mut engine = make_engine("TAKE_PROFIT 5%\nWHEN close > 0\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = vec![candle(100.0), candle(100.0), candle(106.0)];

        engine.on_candle(&candles[..1]).await;
        engine.on_candle(&candles[..2]).await;
        let logs3 = engine.on_candle(&candles[..3]).await;

        assert!(logs3
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::TakeProfitFired { .. })));
        assert!(logs3
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. })));
    }

    #[tokio::test]
    async fn take_profit_does_not_fire_when_gain_under_threshold() {
        // Take profit 10%: open at 100, rise to 105 → 5% gain < 10%.
        let mut engine = make_engine("TAKE_PROFIT 10%\nWHEN close > 0\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = vec![candle(100.0), candle(100.0), candle(105.0)];

        engine.on_candle(&candles[..1]).await;
        engine.on_candle(&candles[..2]).await;
        let logs3 = engine.on_candle(&candles[..3]).await;

        assert!(!logs3
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::TakeProfitFired { .. })));
    }

    #[tokio::test]
    async fn stop_loss_takes_priority_over_take_profit_on_gap() {
        // 2% SL / 5% TP. Open at 100, gap-candle drops to 95 (5% loss) AND
        // would have been a 5% gain if measured the other way — but here
        // close < entry so only SL can fire. The position is closed, and
        // take-profit must NOT fire because the position is gone.
        let mut engine = make_engine(
            "STOP_LOSS 2%\nTAKE_PROFIT 5%\nWHEN close > 0\nBUY 1",
            100_000.0,
        );
        let candles: Vec<Candle> = vec![candle(100.0), candle(95.0)];

        engine.on_candle(&candles[..1]).await;
        let logs2 = engine.on_candle(&candles[..2]).await;

        assert!(logs2
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::StopLossFired { .. })));
        assert!(!logs2
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::TakeProfitFired { .. })));
    }

    #[tokio::test]
    async fn sl_tp_pass_does_nothing_when_no_position() {
        // 2% SL / 5% TP. A dummy rule (which never fires) is needed so the
        // validator accepts the strategy — the engine itself doesn't care
        // about rules, but the validator rejects empty rulesets. Then we
        // assert no SL/TP fires because the rule never opens a position.
        let mut engine = make_engine(
            "STOP_LOSS 2%\nTAKE_PROFIT 5%\nWHEN close > 9999\nBUY 1",
            100_000.0,
        );
        let candles: Vec<Candle> = vec![candle(100.0), candle(50.0), candle(200.0)];

        let logs = engine.on_candle(&candles[..1]).await;
        assert!(!logs
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::StopLossFired { .. })));
        assert!(!logs
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::TakeProfitFired { .. })));
    }

    #[tokio::test]
    async fn strategy_without_sl_tp_does_not_fire_them() {
        // No SL/TP declarations. Position drops hard, no SL/TP logs.
        let mut engine = make_engine("WHEN close > 0\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = vec![candle(100.0), candle(1.0)];

        engine.on_candle(&candles[..1]).await;
        let logs2 = engine.on_candle(&candles[..2]).await;

        assert!(!logs2
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::StopLossFired { .. })));
        assert!(!logs2
            .iter()
            .any(|e| matches!(e.kind, LogEntryKind::TakeProfitFired { .. })));
    }

    #[tokio::test]
    async fn stop_loss_event_carries_loss_pct_and_price() {
        // Verify the log payload: open at 100, close at 96 → 4% loss.
        let mut engine = make_engine("STOP_LOSS 2%\nWHEN close > 0\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = vec![candle(100.0), candle(96.0)];

        engine.on_candle(&candles[..1]).await;
        let logs2 = engine.on_candle(&candles[..2]).await;

        let sl = logs2
            .iter()
            .find_map(|e| match &e.kind {
                LogEntryKind::StopLossFired {
                    symbol,
                    loss_pct,
                    price,
                } => Some((symbol.clone(), *loss_pct, *price)),
                _ => None,
            })
            .expect("expected StopLossFired log entry");
        assert_eq!(sl.0, "TEST");
        assert!((sl.1 - 4.0).abs() < 1e-9, "loss_pct was {}", sl.1);
        assert!((sl.2 - 96.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn take_profit_event_carries_gain_pct_and_price() {
        // Open at 100, close at 108 → 8% gain.
        let mut engine = make_engine("TAKE_PROFIT 5%\nWHEN close > 0\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = vec![candle(100.0), candle(108.0)];

        engine.on_candle(&candles[..1]).await;
        let logs2 = engine.on_candle(&candles[..2]).await;

        let tp = logs2
            .iter()
            .find_map(|e| match &e.kind {
                LogEntryKind::TakeProfitFired {
                    symbol,
                    gain_pct,
                    price,
                } => Some((symbol.clone(), *gain_pct, *price)),
                _ => None,
            })
            .expect("expected TakeProfitFired log entry");
        assert_eq!(tp.0, "TEST");
        assert!((tp.1 - 8.0).abs() < 1e-9, "gain_pct was {}", tp.1);
        assert!((tp.2 - 108.0).abs() < 1e-9);
    }

    #[tokio::test]
    async fn sl_tp_is_deterministic() {
        // Same source + same candles must produce the same SL/TP log
        // ordering. Backtest determinism invariant.
        let source = "STOP_LOSS 2%\nTAKE_PROFIT 5%\nWHEN close > 0\nBUY 1";
        let candles: Vec<Candle> = (90..=110).map(|c| candle(c as f64)).collect();

        let mut e1 = make_engine(source, 100_000.0);
        let mut e2 = make_engine(source, 100_000.0);

        let mut all1 = Vec::new();
        let mut all2 = Vec::new();
        for index in 1..=candles.len() {
            all1.extend(e1.on_candle(&candles[..index]).await);
            all2.extend(e2.on_candle(&candles[..index]).await);
        }

        let kind = |e: &LogEntry| match &e.kind {
            LogEntryKind::OrderExecuted { .. } => "exec".to_string(),
            LogEntryKind::StopLossFired { .. } => "sl".to_string(),
            LogEntryKind::TakeProfitFired { .. } => "tp".to_string(),
            LogEntryKind::RuleFired { .. } => "fired".to_string(),
            LogEntryKind::OrderSubmitted { .. } => "submit".to_string(),
            _ => "other".to_string(),
        };
        let seq1: Vec<String> = all1.iter().map(kind).collect();
        let seq2: Vec<String> = all2.iter().map(kind).collect();
        assert_eq!(seq1, seq2);
    }

    #[tokio::test]
    async fn stop_loss_fires_every_candle_while_underwater() {
        // SL is NOT edge-triggered. It must fire every candle the position
        // is underwater, not just on the transition candle. We expect one
        // SL fire per candle the position is open and underwater.
        let mut engine = make_engine("STOP_LOSS 2%\nWHEN close > 0\nBUY 1", 100_000.0);
        // Open at 100, then stay at 97 (3% loss) for three more candles.
        let candles: Vec<Candle> = vec![
            candle(100.0),
            candle(97.0),
            candle(97.0),
            candle(97.0),
        ];

        let _ = engine.on_candle(&candles[..1]).await;
        let mut total_sl = 0;
        for index in 2..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_sl += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::StopLossFired { .. }))
                .count();
        }
        // Position closes on candle 2's SL; candles 3 and 4 have no
        // position, so no SL fires.
        assert_eq!(total_sl, 1);
    }

    // ---------- RISK ----------

    #[tokio::test]
    async fn risk_max_orders_blocks_after_limit() {
        // MAX_ORDERS 2: first two candles fire (close rises above 100, then
        // stays above 100 → trigger holds → re-fires after drop). With the
        // rule re-armed only after close drops below 105, the third
        // qualifying candle must be skipped.
        let mut engine =
            make_engine("RISK MAX_ORDERS 2\nWHEN close > 100\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = vec![
            candle(100.0),
            candle(110.0),
            candle(50.0),
            candle(110.0),
            candle(50.0),
            candle(110.0),
        ];

        let mut total_orders = 0;
        let mut total_breaches = 0;
        for index in 1..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_orders += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. }))
                .count();
            total_breaches += logs
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        LogEntryKind::RiskBreach {
                            reason: RiskBreachReason::MaxOrdersReached,
                            ..
                        }
                    )
                })
                .count();
        }
        assert_eq!(total_orders, 2);
        assert_eq!(total_breaches, 1);
    }

    #[tokio::test]
    async fn risk_max_orders_does_not_count_failed_orders() {
        // MAX_ORDERS 1: alternating up/down so the rule re-fires every
        // other candle. The first up-candle's BUY fails (insufficient
        // cash: 1 share @ 200 = 200 > 100 cash), so it does NOT count
        // toward the limit. The next up-candle's BUY succeeds (price
        // 80) and is the first counted order; the third up-candle is
        // blocked.
        let mut engine =
            make_engine("RISK MAX_ORDERS 1\nWHEN close > 50\nBUY 1", 100.0);
        let candles: Vec<Candle> = vec![
            candle(40.0),  // close <= 50: rule doesn't fire
            candle(200.0), // fires BUY → fails (cost 200 > cash 100)
            candle(40.0),  // close <= 50: rule doesn't fire, trigger resets
            candle(80.0),  // fires BUY → succeeds (count = 1)
            candle(40.0),  // close <= 50: rule doesn't fire, trigger resets
            candle(80.0),  // limit reached → blocked
        ];

        let mut total_orders = 0;
        let mut total_failures = 0;
        let mut total_breaches = 0;
        for index in 1..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_orders += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. }))
                .count();
            total_failures += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::OrderFailed { .. }))
                .count();
            total_breaches += logs
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        LogEntryKind::RiskBreach {
                            reason: RiskBreachReason::MaxOrdersReached,
                            ..
                        }
                    )
                })
                .count();
        }
        assert_eq!(total_orders, 1);
        assert_eq!(total_failures, 1);
        assert_eq!(total_breaches, 1);
    }

    #[tokio::test]
    async fn risk_max_positions_blocks_buys_when_at_limit() {
        // MAX_POSITIONS 1 + alternating up/down so the rule re-fires.
        // The first up-candle's BUY opens a position. The next up-candle
        // (after a reset) must be blocked because the first position is
        // still open.
        let mut engine = make_engine(
            "RISK MAX_POSITIONS 1\nWHEN close > 100\nBUY 1",
            100_000.0,
        );
        let candles: Vec<Candle> = vec![
            candle(50.0),  // rule doesn't fire
            candle(150.0), // fires BUY → succeeds
            candle(50.0),  // rule doesn't fire, trigger resets
            candle(150.0), // blocked: 1 position already open
        ];

        let mut total_orders = 0;
        let mut total_breaches = 0;
        for index in 1..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_orders += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. }))
                .count();
            total_breaches += logs
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        LogEntryKind::RiskBreach {
                            reason: RiskBreachReason::MaxPositionsReached,
                            ..
                        }
                    )
                })
                .count();
        }
        assert_eq!(total_orders, 1, "only the first BUY should execute");
        assert_eq!(total_breaches, 1);
    }

    #[tokio::test]
    async fn risk_max_positions_does_not_block_sells() {
        // MAX_POSITIONS 1: open a position, then a TP-like rule fires a
        // SELL ALL on the second candle. MAX_POSITIONS must not block the
        // sell even though there is an open position.
        let mut engine = make_engine(
            "RISK MAX_POSITIONS 1\nWHEN close > 100\nBUY 1\nWHEN close > 105\nSELL ALL",
            100_000.0,
        );
        let candles: Vec<Candle> = vec![
            candle(50.0),  // rules don't fire
            candle(110.0), // close > 100 → BUY, close > 105 → SELL ALL
        ];

        let mut total_orders = 0;
        let mut total_breaches = 0;
        for index in 1..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_orders += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. }))
                .count();
            total_breaches += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::RiskBreach { .. }))
                .count();
        }
        // 1 BUY + 1 SELL = 2 successful orders; no breaches (sells aren't
        // checked, and the BUY happens before MAX_POSITIONS would have a
        // chance to compare to itself).
        assert_eq!(total_orders, 2);
        assert_eq!(total_breaches, 0);
    }

    #[tokio::test]
    async fn risk_max_daily_loss_blocks_after_threshold() {
        // MAX_DAILY_LOSS 1% with initial_cash 1000 (so 1% = 10 realized
        // loss). BUY fires on `close > 100`, SELL on `close < 50`.
        // Open at 200, drop to 40, SELL ALL — realized loss = 160 (160/1000
        // = 16% > 1%). Next buy cycle (after the trigger re-arms) is
        // blocked by the threshold.
        let mut engine = make_engine(
            "RISK MAX_DAILY_LOSS 1%\nWHEN close > 100\nBUY 1\nWHEN close < 50\nSELL ALL",
            1000.0,
        );
        let candles: Vec<Candle> = vec![
            candle(80.0),  // no rule fires
            candle(200.0), // close>100 → BUY 1 @ 200 (cost 200, OK)
            candle(40.0),  // close<50 → SELL ALL @ 40 (loss = 160)
            candle(200.0), // close>100 → BUY blocked (16% loss > 1%)
        ];

        let mut total_orders = 0;
        let mut total_breaches = 0;
        for index in 1..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_orders += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. }))
                .count();
            total_breaches += logs
                .iter()
                .filter(|e| {
                    matches!(
                        e.kind,
                        LogEntryKind::RiskBreach {
                            reason: RiskBreachReason::MaxDailyLossReached,
                            ..
                        }
                    )
                })
                .count();
        }
        assert_eq!(total_orders, 2, "two orders: BUY + SELL");
        assert!(total_breaches >= 1, "expected at least one breach");
    }

    #[tokio::test]
    async fn risk_breach_does_not_increment_session_orders() {
        // MAX_ORDERS 1 with alternating up/down. The first up-candle's
        // BUY succeeds (count = 1). The second up-candle's BUY is
        // blocked; the breach must NOT count toward the limit.
        let mut engine =
            make_engine("RISK MAX_ORDERS 1\nWHEN close > 100\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = vec![
            candle(50.0),  // rule doesn't fire
            candle(150.0), // fires BUY → succeeds (count = 1)
            candle(50.0),  // rule doesn't fire, trigger resets
            candle(150.0), // limit reached → blocked (breach)
        ];

        let mut total_orders = 0;
        let mut total_breaches = 0;
        for index in 1..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_orders += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. }))
                .count();
            total_breaches += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::RiskBreach { .. }))
                .count();
        }
        assert_eq!(total_orders, 1);
        assert_eq!(total_breaches, 1);
    }

    #[tokio::test]
    async fn strategy_without_risk_does_not_log_breaches() {
        // No RISK declarations: even with the rule firing every candle, no
        // RiskBreach entries are emitted and every order executes.
        let mut engine = make_engine("WHEN close > 0\nBUY 1", 100_000.0);
        let candles: Vec<Candle> = vec![candle(10.0), candle(20.0), candle(30.0)];

        let mut total_orders = 0;
        let mut total_breaches = 0;
        for index in 1..=candles.len() {
            let logs = engine.on_candle(&candles[..index]).await;
            total_orders += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::OrderExecuted { .. }))
                .count();
            total_breaches += logs
                .iter()
                .filter(|e| matches!(e.kind, LogEntryKind::RiskBreach { .. }))
                .count();
        }
        assert!(total_orders > 0);
        assert_eq!(total_breaches, 0);
    }

    #[tokio::test]
    async fn risk_is_deterministic() {
        // Same source + same candles = same RiskBreach log sequence.
        // Backtest determinism invariant must hold with risk controls on.
        let source = "RISK MAX_ORDERS 2\nWHEN close > 0\nBUY 1";
        let candles: Vec<Candle> = (1..=10).map(|c| candle(c as f64)).collect();

        let mut e1 = make_engine(source, 100_000.0);
        let mut e2 = make_engine(source, 100_000.0);

        let mut all1 = Vec::new();
        let mut all2 = Vec::new();
        for index in 1..=candles.len() {
            all1.extend(e1.on_candle(&candles[..index]).await);
            all2.extend(e2.on_candle(&candles[..index]).await);
        }

        let kind = |e: &LogEntry| match &e.kind {
            LogEntryKind::OrderExecuted { .. } => "exec".to_string(),
            LogEntryKind::OrderSubmitted { .. } => "submit".to_string(),
            LogEntryKind::RiskBreach { reason, .. } => format!("breach:{reason:?}"),
            _ => "other".to_string(),
        };
        let seq1: Vec<String> = all1.iter().map(kind).collect();
        let seq2: Vec<String> = all2.iter().map(kind).collect();
        assert_eq!(seq1, seq2);
    }
}
