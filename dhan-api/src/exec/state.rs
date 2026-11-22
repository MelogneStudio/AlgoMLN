use crate::exec::{
    error::ErrorClass,
    normalize::{NormalizedOrderUpdate, NormalizedStatus},
    types::{ExecEvent, LocalOrderId, OrderRequest},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderState {
    Created,
    Submitting,
    Submitted,
    Open,
    PartiallyFilled,
    Filled,
    Rejected,
    Cancelled,
    Expired,
    Unknown,
}

impl OrderState {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            OrderState::Filled
                | OrderState::Rejected
                | OrderState::Cancelled
                | OrderState::Expired
        )
    }

    pub fn is_open_or_pending(&self) -> bool {
        matches!(
            self,
            OrderState::Created
                | OrderState::Submitting
                | OrderState::Submitted
                | OrderState::Open
                | OrderState::PartiallyFilled
        )
    }
}

/// Tracks the full lifecycle and fill accounting for a single order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OrderTracker {
    pub local_id: LocalOrderId,
    pub correlation_id: String,
    pub dhan_order_id: Option<String>,
    pub request: OrderRequest,
    pub state: OrderState,
    pub order_qty: u64,
    pub cum_traded_qty: u64,
    pub remaining_qty: u64,
    pub seen_trade_ids: BTreeSet<String>,
    pub last_avg_price_paise: Option<i64>,
}

impl OrderTracker {
    pub fn new(req: OrderRequest, correlation_id: String) -> Self {
        let order_qty = req.quantity;
        Self {
            local_id: req.local_id.clone(),
            correlation_id,
            dhan_order_id: None,
            request: req,
            state: OrderState::Created,
            order_qty,
            cum_traded_qty: 0,
            remaining_qty: order_qty,
            seen_trade_ids: BTreeSet::new(),
            last_avg_price_paise: None,
        }
    }

    pub fn mark_submitting(&mut self) {
        if self.state == OrderState::Created {
            self.state = OrderState::Submitting;
        }
    }

    pub fn mark_submitted(&mut self, dhan_order_id: Option<String>) -> ExecEvent {
        self.state = OrderState::Submitted;
        if dhan_order_id.is_some() {
            self.dhan_order_id = dhan_order_id.clone();
        }
        ExecEvent::Submitted {
            local_id: self.local_id.clone(),
            dhan_order_id,
        }
    }

    /// Process a fill trade event (deduped by exchange_trade_id).
    /// Returns Some(ExecEvent::Fill) if new, None if duplicate.
    pub fn record_trade_fill(
        &mut self,
        exchange_trade_id: &str,
        fill_qty: u64,
        fill_price_paise: i64,
    ) -> Option<ExecEvent> {
        if self.seen_trade_ids.contains(exchange_trade_id) {
            return None; // Deduped, do not double-count
        }
        self.seen_trade_ids.insert(exchange_trade_id.to_string());
        Some(ExecEvent::Fill {
            local_id: self.local_id.clone(),
            qty: fill_qty,
            price_paise: fill_price_paise,
            exchange_trade_id: exchange_trade_id.to_string(),
        })
    }

    /// Apply a normalized update from REST poll or WebSocket feed.
    /// Emits all resulting events in order.
    pub fn apply_update(&mut self, update: &NormalizedOrderUpdate) -> Vec<ExecEvent> {
        let mut events = Vec::new();

        // 1. Terminal state absorption: late events are absorbed/ignored
        if self.state.is_terminal() {
            return events;
        }

        // Store dhan_order_id if present
        if self.dhan_order_id.is_none() && update.dhan_order_id.is_some() {
            self.dhan_order_id = update.dhan_order_id.clone();
        }

        // Store average price if present
        if update.avg_price_paise.is_some() {
            self.last_avg_price_paise = update.avg_price_paise;
        }

        // 2. Monotonicity check: cumulative traded quantity never decreases
        if update.cum_traded_qty < self.cum_traded_qty {
            self.state = OrderState::Unknown;
            events.push(ExecEvent::Unknown {
                local_id: self.local_id.clone(),
                reason: format!(
                    "cumulative traded quantity decreased from {} to {}",
                    self.cum_traded_qty, update.cum_traded_qty
                ),
            });
            return events;
        }

        // 3. Incremental fill accounting
        let incremental_qty = update.cum_traded_qty.saturating_sub(self.cum_traded_qty);
        if incremental_qty > 0 {
            self.cum_traded_qty = update.cum_traded_qty;
            self.remaining_qty = self.order_qty.saturating_sub(self.cum_traded_qty);

            // Price for fill: prefer last_traded_price, then avg_price, then limit price, then 0
            let price_paise = update
                .last_traded_price_paise
                .or(update.avg_price_paise)
                .or_else(|| match self.request.kind {
                    crate::exec::types::OrderKind::Limit { price_paise } => Some(price_paise),
                    crate::exec::types::OrderKind::Market => None,
                })
                .unwrap_or(0);

            // Generate synthetic exchange trade id if this was from cumulative diff
            let synth_trade_id = format!(
                "synth-{}-{}",
                self.cum_traded_qty,
                update.update_time.as_deref().unwrap_or("now")
            );

            if let Some(fill_event) = self.record_trade_fill(&synth_trade_id, incremental_qty, price_paise) {
                events.push(fill_event);
            }
        }

        // 4. Status mapping & transitions
        match &update.status {
            NormalizedStatus::Submitted => {
                if self.state == OrderState::Created || self.state == OrderState::Submitting {
                    self.state = OrderState::Submitted;
                    events.push(ExecEvent::Submitted {
                        local_id: self.local_id.clone(),
                        dhan_order_id: self.dhan_order_id.clone(),
                    });
                }
            }
            NormalizedStatus::Open => {
                if self.state != OrderState::Open {
                    self.state = OrderState::Open;
                    events.push(ExecEvent::Open {
                        local_id: self.local_id.clone(),
                        dhan_order_id: self.dhan_order_id.clone().unwrap_or_default(),
                    });
                }
            }
            NormalizedStatus::PartiallyFilled => {
                self.state = OrderState::PartiallyFilled;
            }
            NormalizedStatus::Filled => {
                self.state = OrderState::Filled;
                self.remaining_qty = 0;
                events.push(ExecEvent::Filled {
                    local_id: self.local_id.clone(),
                });
            }
            NormalizedStatus::Cancelled => {
                self.state = OrderState::Cancelled;
                events.push(ExecEvent::Cancelled {
                    local_id: self.local_id.clone(),
                    remaining_qty: self.remaining_qty,
                });
            }
            NormalizedStatus::Rejected => {
                self.state = OrderState::Rejected;
                let reason = update
                    .reject_reason
                    .clone()
                    .unwrap_or_else(|| "order rejected by broker".to_string());
                events.push(ExecEvent::Rejected {
                    local_id: self.local_id.clone(),
                    reason,
                    error_class: ErrorClass::BusinessRejection,
                });
            }
            NormalizedStatus::Expired => {
                self.state = OrderState::Expired;
                events.push(ExecEvent::Expired {
                    local_id: self.local_id.clone(),
                    remaining_qty: self.remaining_qty,
                });
            }
            NormalizedStatus::ClosedLogOnly => {
                // Ignore super orders CLOSED status
            }
            NormalizedStatus::Unknown(raw) => {
                self.state = OrderState::Unknown;
                events.push(ExecEvent::Unknown {
                    local_id: self.local_id.clone(),
                    reason: format!("unrecognized broker order status: '{raw}'"),
                });
            }
        }

        events
    }
}
