use async_trait::async_trait;
use serde::Serialize;

use crate::models::{Order, OrderResult, Position};

#[async_trait]
pub trait ExecutionTarget: Send + Sync {
    async fn execute(&self, order: Order) -> Result<OrderResult, ExecutionError>;
    async fn get_positions(&self) -> Result<Vec<Position>, ExecutionError>;
    /// Sum of negative realized `PaperTrade.pnl` values, i.e. the total
    /// realized loss as a positive number. Returns 0.0 if no loss has been
    /// realized (or if the broker is net positive on the session). Used by
    /// the engine's `RISK MAX_DAILY_LOSS` check.
    fn realized_loss(&self) -> f64;
    fn available_cash(&self) -> f64;
    fn is_paper(&self) -> bool;
    fn name(&self) -> &str;
    /// Downcast hook so the engine can recover concrete broker state
    /// (e.g. the `PaperBroker`'s `trade_history` to publish a
    /// `TradeExecuted` event). Mirrors the `as_any` pattern on
    /// `IndicatorRegistryApi` / `UiApi`.
    fn as_any(&self) -> &dyn std::any::Any;
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecutionError {
    pub message: String,
    pub kind: ExecutionErrorKind,
}

#[derive(Debug, Clone, Serialize)]
pub enum ExecutionErrorKind {
    InsufficientFunds,
    InsufficientPosition,
    BrokerError(String),
}

impl ExecutionError {
    pub fn insufficient_funds(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ExecutionErrorKind::InsufficientFunds,
        }
    }

    pub fn insufficient_position(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ExecutionErrorKind::InsufficientPosition,
        }
    }
}
