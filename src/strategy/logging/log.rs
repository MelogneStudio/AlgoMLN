use serde::Serialize;

use crate::models::{Order, OrderResult};
use crate::strategy::dsl::{ActionNode, IndicatorKind};
use crate::strategy::runtime::StrategyStatus;

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub id: String,
    pub timestamp: i64,
    pub strategy_id: String,
    pub candle_timestamp: i64,
    pub kind: LogEntryKind,
}

#[derive(Debug, Clone, Serialize)]
pub enum LogEntryKind {
    ConditionEvaluated {
        rule_id: String,
        result: bool,
        prev_state: bool,
        fired: bool,
        indicator_snapshots: Vec<IndicatorSnapshot>,
    },
    RuleFired {
        rule_id: String,
        action: ActionNode,
    },
    OrderSubmitted {
        rule_id: String,
        order: Order,
    },
    OrderExecuted {
        rule_id: String,
        result: OrderResult,
    },
    RuleSkipped {
        rule_id: String,
        reason: RuleSkipReason,
    },
    OrderFailed {
        rule_id: String,
        error: String,
    },
    EvalError {
        rule_id: String,
        error: String,
    },
    StatusChanged {
        from: StrategyStatus,
        to: StrategyStatus,
    },
    /// Stop-loss threshold was breached for a held position. Logged before
    /// the resulting `SELL ALL` order is submitted. `symbol` is the held
    /// symbol, `loss_pct` is the unrealized loss against the entry price
    /// (e.g. 2.5 means the position is 2.5% underwater), `price` is the
    /// candle close that triggered the breach.
    StopLossFired {
        symbol: String,
        loss_pct: f64,
        price: f64,
    },
    /// Take-profit threshold was breached for a held position. Logged
    /// before the resulting `SELL ALL` order is submitted. `gain_pct` is
    /// the unrealized gain against the entry price (e.g. 5.0 means 5%
    /// above entry). When both SL and TP would fire on the same candle,
    /// the stop-loss fires first and take-profit is skipped.
    TakeProfitFired {
        symbol: String,
        gain_pct: f64,
        price: f64,
    },
    /// A risk-control declaration on the strategy (e.g. `RISK MAX_ORDERS`)
    /// blocked this order. Logged instead of submitting the order; the
    /// engine does not surface an error to the backtest orchestrator.
    RiskBreach {
        rule_id: String,
        reason: RiskBreachReason,
    },
}

#[derive(Debug, Clone, Serialize)]
pub enum RuleSkipReason {
    NoPosition,
    InsufficientCash,
}

/// Why a risk-control check blocked an order. The variants mirror the three
/// `RISK` declarations: `RISK MAX_ORDERS`, `RISK MAX_POSITIONS`, and
/// `RISK MAX_DAILY_LOSS`. `MaxDailyLossReached` is a hard stop — once the
/// cumulative realized loss crosses the threshold, every subsequent order
/// is skipped regardless of side.
#[derive(Debug, Clone, Serialize)]
pub enum RiskBreachReason {
    MaxOrdersReached,
    MaxPositionsReached,
    MaxDailyLossReached,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndicatorSnapshot {
    pub kind: IndicatorKind,
    pub period: usize,
    pub value: f64,
}

#[derive(Debug)]
pub struct StrategyLogger {
    strategy_id: String,
    entries: Vec<LogEntry>,
    next_id: usize,
}

impl StrategyLogger {
    pub fn new(strategy_id: String) -> Self {
        Self {
            strategy_id,
            entries: Vec::new(),
            next_id: 0,
        }
    }

    pub fn log(&mut self, kind: LogEntryKind, candle_timestamp: i64) {
        let id = format!("log_{}", self.next_id);
        self.next_id += 1;
        self.entries.push(LogEntry {
            id,
            timestamp: candle_timestamp,
            strategy_id: self.strategy_id.clone(),
            candle_timestamp,
            kind,
        });
    }

    pub fn get_entries(&self) -> &[LogEntry] {
        &self.entries
    }

    pub fn drain_entries(&mut self) -> Vec<LogEntry> {
        self.entries.drain(..).collect()
    }
}
