use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrderType {
    Market,
    Limit,
    StopLoss,
}

/// Lifecycle status of an order as reported by the broker. `Transit` and
/// `Pending` are **non-terminal** — the order has been submitted but not yet
/// filled, so downstream code must inspect [`OrderStatus::is_fill`] before
/// treating it as an execution. `Unknown` carries whatever raw status string
/// the broker sent that we don't model explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OrderStatus {
    /// Submitted to the broker, not yet at the exchange. Non-terminal.
    Transit,
    /// At the exchange, awaiting a fill. Non-terminal.
    Pending,
    /// Filled (fully or partially). Terminal.
    Traded,
    /// Rejected at the broker or exchange. Terminal.
    Rejected,
    /// Cancelled. Terminal.
    Cancelled,
    /// Expired without filling. Terminal.
    Expired,
    /// Any status string the broker sent that we don't model. Terminal is
    /// deliberately `false` so unknown statuses are never mistaken for a fill.
    Unknown(String),
}

impl OrderStatus {
    /// True if the order has reached a final state (filled, rejected,
    /// cancelled, or expired). `Transit`, `Pending`, and `Unknown` are treated
    /// as non-terminal.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Traded | Self::Rejected | Self::Cancelled | Self::Expired
        )
    }

    /// True only if the order actually filled. Everything else — including
    /// non-terminal and unknown statuses — is not a fill.
    pub fn is_fill(&self) -> bool {
        matches!(self, Self::Traded)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    pub symbol: String,
    pub side: OrderSide,
    pub quantity: u32,
    pub order_type: OrderType,
    pub price: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OrderResult {
    pub order_id: String,
    pub status: OrderStatus,
    pub timestamp: i64,
    /// Idempotency correlation id sent to the broker with this order. Empty for
    /// brokers that don't use one (e.g. the paper broker).
    pub correlation_id: String,
}

#[cfg(test)]
mod tests {
    use super::OrderStatus;

    #[test]
    fn test_transit_is_not_terminal() {
        assert!(!OrderStatus::Transit.is_terminal());
        assert!(!OrderStatus::Transit.is_fill());
    }

    #[test]
    fn test_pending_is_not_terminal() {
        assert!(!OrderStatus::Pending.is_terminal());
        assert!(!OrderStatus::Pending.is_fill());
    }

    #[test]
    fn test_traded_is_terminal_and_fill() {
        assert!(OrderStatus::Traded.is_terminal());
        assert!(OrderStatus::Traded.is_fill());
    }

    #[test]
    fn test_terminal_non_fills() {
        for status in [
            OrderStatus::Rejected,
            OrderStatus::Cancelled,
            OrderStatus::Expired,
        ] {
            assert!(status.is_terminal());
            assert!(!status.is_fill());
        }
    }

    #[test]
    fn test_unknown_is_not_terminal_and_not_fill() {
        let status = OrderStatus::Unknown("WEIRD".to_string());
        assert!(!status.is_terminal());
        assert!(!status.is_fill());
    }
}
