use crate::models::{Order, OrderSide, OrderType};
use crate::strategy::dsl::ActionNode;

use super::paper::PaperPosition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderBuildError {
    NoPosition,
    ZeroQuantity,
    QuantityTooLarge,
}

pub fn build_order(
    action: &ActionNode,
    symbol: &str,
    current_price: f64,
    current_position: Option<&PaperPosition>,
    _rule_id: &str,
) -> Result<Order, OrderBuildError> {
    let (side, quantity) = match action {
        ActionNode::Buy { quantity } => (OrderSide::Buy, *quantity),
        ActionNode::Sell { quantity } => (OrderSide::Sell, *quantity),
        ActionNode::SellAll => {
            let position = current_position.ok_or(OrderBuildError::NoPosition)?;
            if position.quantity <= 0 {
                return Err(OrderBuildError::NoPosition);
            }
            (OrderSide::Sell, position.quantity as usize)
        }
    };

    if quantity == 0 {
        return Err(OrderBuildError::ZeroQuantity);
    }

    let quantity = u32::try_from(quantity).map_err(|_| OrderBuildError::QuantityTooLarge)?;

    Ok(Order {
        symbol: symbol.to_string(),
        side,
        quantity,
        order_type: OrderType::Market,
        price: Some(current_price),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sell_all_requires_position() {
        let err = build_order(&ActionNode::SellAll, "NIFTY", 100.0, None, "rule_0").unwrap_err();
        assert_eq!(err, OrderBuildError::NoPosition);
    }

    #[test]
    fn sell_all_uses_full_position_quantity() {
        let position = PaperPosition {
            symbol: "NIFTY".to_string(),
            quantity: 7,
            avg_entry_price: 90.0,
            unrealized_pnl: 0.0,
        };
        let order =
            build_order(&ActionNode::SellAll, "NIFTY", 100.0, Some(&position), "rule_0").unwrap();
        assert_eq!(order.quantity, 7);
        assert_eq!(order.side, OrderSide::Sell);
    }
}
