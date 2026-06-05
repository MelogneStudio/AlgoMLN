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
}

#[derive(Debug, Clone, Serialize)]
pub enum RuleSkipReason {
    NoPosition,
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
