use crate::models::{Order, OrderSide, OrderType};
use crate::strategy::dsl::{ActionNode, QuantitySpec};

use super::paper::PaperPosition;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderBuildError {
    NoPosition,
    InsufficientCash,
    ZeroQuantity,
    QuantityTooLarge,
}

pub fn build_order(
    action: &ActionNode,
    symbol: &str,
    current_price: f64,
    available_cash: f64,
    current_position: Option<&PaperPosition>,
    _rule_id: &str,
) -> Result<Order, OrderBuildError> {
    let (side, quantity) = match action {
        ActionNode::Buy { quantity } => {
            let quantity = resolve_quantity(quantity, current_price, available_cash)?;
            (OrderSide::Buy, quantity)
        }
        ActionNode::Sell { quantity } => {
            let quantity = resolve_quantity(quantity, current_price, available_cash)?;
            (OrderSide::Sell, quantity)
        }
        ActionNode::SellAll => {
            let position = current_position.ok_or(OrderBuildError::NoPosition)?;
            if position.quantity <= 0 {
                return Err(OrderBuildError::NoPosition);
            }
            (OrderSide::Sell, position.quantity as u64)
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

fn resolve_quantity(
    quantity: &QuantitySpec,
    current_price: f64,
    available_cash: f64,
) -> Result<u64, OrderBuildError> {
    match quantity {
        QuantitySpec::Fixed(value) => Ok(*value),
        QuantitySpec::PercentCapital(pct) => {
            let rupees = available_cash * pct / 100.0;
            quantity_from_rupees(rupees, current_price)
        }
        QuantitySpec::ValueBased(value) => quantity_from_rupees(*value, current_price),
    }
}

fn quantity_from_rupees(rupees: f64, current_price: f64) -> Result<u64, OrderBuildError> {
    if current_price <= 0.0 {
        return Err(OrderBuildError::InsufficientCash);
    }
    let quantity = (rupees / current_price).floor() as u64;
    if quantity == 0 {
        return Err(OrderBuildError::InsufficientCash);
    }
    Ok(quantity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sell_all_requires_position() {
        let err = build_order(
            &ActionNode::SellAll,
            "NIFTY",
            100.0,
            1_000.0,
            None,
            "rule_0",
        )
        .unwrap_err();
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
        let order = build_order(
            &ActionNode::SellAll,
            "NIFTY",
            100.0,
            1_000.0,
            Some(&position),
            "rule_0",
        )
        .unwrap();
        assert_eq!(order.quantity, 7);
        assert_eq!(order.side, OrderSide::Sell);
    }

    #[test]
    fn percent_quantity_uses_available_cash() {
        let order = build_order(
            &ActionNode::Buy {
                quantity: QuantitySpec::PercentCapital(10.0),
            },
            "NIFTY",
            100.0,
            1_000.0,
            None,
            "rule_0",
        )
        .unwrap();
        assert_eq!(order.quantity, 1);
    }

    #[test]
    fn value_based_quantity_uses_current_price() {
        let order = build_order(
            &ActionNode::Buy {
                quantity: QuantitySpec::ValueBased(550.0),
            },
            "NIFTY",
            100.0,
            1_000.0,
            None,
            "rule_0",
        )
        .unwrap();
        assert_eq!(order.quantity, 5);
    }
}
