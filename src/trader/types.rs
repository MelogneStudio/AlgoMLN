use crate::engine::{RuleId, Symbol};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type OrderId = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Act,
    Observe,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderReason {
    Rule(RuleId),
    StopLoss,
    TakeProfit,
    DailyLossFlatten,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderRequest {
    pub client_order_id: OrderId,
    pub symbol: Symbol,
    pub side: Side,
    pub qty: u64,
    pub reason: OrderReason,
    pub ref_price: f64,
    pub decided_at: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum OrderEvent {
    Acked {
        id: OrderId,
    },
    PartialFill {
        id: OrderId,
        qty: u64,
        price: f64,
        fees: f64,
    },
    Filled {
        id: OrderId,
        qty: u64,
        price: f64,
        fees: f64,
    },
    Rejected {
        id: OrderId,
        reason: String,
    },
    Cancelled {
        id: OrderId,
    },
    Expired {
        id: OrderId,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortError {
    Message(String),
    UnknownOrder(OrderId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstrumentRule {
    pub lot_size: u64,
    pub min_qty: u64,
}

impl Default for InstrumentRule {
    fn default() -> Self {
        Self {
            lot_size: 1,
            min_qty: 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TraderConfig {
    pub initial_cash: f64,
    pub instrument_rules: BTreeMap<Symbol, InstrumentRule>,
}

impl Default for TraderConfig {
    fn default() -> Self {
        Self {
            initial_cash: 0.0,
            instrument_rules: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraderError {
    InitialCashRequired,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum TraderEvent {
    RuleSkipped {
        symbol: Symbol,
        rule_id: RuleId,
    },
    RiskBlocked {
        symbol: Symbol,
        rule_id: RuleId,
        limit: &'static str,
    },
    DailyLossBreached,
    OrderEmitted(OrderRequest),
    Fill {
        id: OrderId,
        qty: u64,
        price: f64,
        fees: f64,
    },
    OrderRejected {
        id: OrderId,
        reason: String,
    },
    OrderClosed {
        id: OrderId,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameOutput {
    pub orders: Vec<OrderRequest>,
    /// Broker order IDs the adapter should cancel after this frame.
    pub cancels: Vec<OrderId>,
    pub events: Vec<TraderEvent>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Position {
    pub filled_qty: i64,
    pub avg_entry_paise: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderState {
    Submitted,
    Acked,
    PartialFill,
    Filled,
    Rejected,
    Cancelled,
    Expired,
}

#[derive(Clone, Debug)]
pub(crate) struct Pending {
    pub(crate) req: OrderRequest,
    pub(crate) filled: u64,
    pub(crate) state: OrderState,
    pub(crate) cancel_requested: bool,
}

impl Pending {
    pub(crate) fn live(&self) -> bool {
        matches!(
            self.state,
            OrderState::Submitted | OrderState::Acked | OrderState::PartialFill
        )
    }

    pub(crate) fn remaining(&self) -> u64 {
        self.req.qty.saturating_sub(self.filled)
    }
}

pub(crate) fn event_id(e: &OrderEvent) -> &str {
    match e {
        OrderEvent::Acked { id }
        | OrderEvent::PartialFill { id, .. }
        | OrderEvent::Filled { id, .. }
        | OrderEvent::Rejected { id, .. }
        | OrderEvent::Cancelled { id }
        | OrderEvent::Expired { id } => id,
    }
}
