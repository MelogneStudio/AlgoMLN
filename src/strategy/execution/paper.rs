use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use serde::Serialize;

use crate::models::{Order, OrderResult, OrderSide, OrderStatus, Position};

use super::target::{ExecutionError, ExecutionTarget};

#[derive(Debug)]
pub struct PaperBroker {
    pub symbol: String,
    state: Mutex<PaperBrokerInner>,
}

#[derive(Debug)]
struct PaperBrokerInner {
    cash: f64,
    initial_cash: f64,
    positions: HashMap<String, PaperPosition>,
    trade_history: Vec<PaperTrade>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaperPosition {
    pub symbol: String,
    pub quantity: i64,
    pub avg_entry_price: f64,
    pub unrealized_pnl: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaperTrade {
    pub id: String,
    pub timestamp: i64,
    pub symbol: String,
    pub side: OrderSide,
    pub quantity: usize,
    pub price: f64,
    pub rule_id: String,
    pub pnl: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PaperBrokerState {
    pub cash: f64,
    pub initial_cash: f64,
    pub positions: Vec<PaperPosition>,
    pub trade_history: Vec<PaperTrade>,
    pub total_realized_pnl: f64,
}

impl PaperBroker {
    pub fn new(symbol: String, initial_cash: f64) -> Self {
        Self {
            symbol,
            state: Mutex::new(PaperBrokerInner {
                cash: initial_cash,
                initial_cash,
                positions: HashMap::new(),
                trade_history: Vec::new(),
            }),
        }
    }

    pub fn get_state(&self) -> PaperBrokerState {
        let state = self.state.lock().expect("paper broker mutex poisoned");
        broker_state(&state)
    }

    pub fn get_position(&self, symbol: &str) -> Option<PaperPosition> {
        self.state
            .lock()
            .expect("paper broker mutex poisoned")
            .positions
            .get(symbol)
            .cloned()
    }

    pub fn update_unrealized(&self, symbol: &str, current_price: f64) {
        let mut state = self.state.lock().expect("paper broker mutex poisoned");
        if let Some(position) = state.positions.get_mut(symbol) {
            position.unrealized_pnl =
                (current_price - position.avg_entry_price) * position.quantity as f64;
        }
    }

    pub fn reset(&self) {
        let mut state = self.state.lock().expect("paper broker mutex poisoned");
        state.cash = state.initial_cash;
        state.positions.clear();
        state.trade_history.clear();
    }

    fn execute_locked(
        &self,
        state: &mut PaperBrokerInner,
        order: Order,
    ) -> Result<OrderResult, ExecutionError> {
        let price = order.price.unwrap_or(0.0);
        let quantity = order.quantity as usize;

        match order.side {
            OrderSide::Buy => execute_buy(state, &order.symbol, quantity, price)?,
            OrderSide::Sell => execute_sell(state, &order.symbol, quantity, price)?,
        }

        Ok(OrderResult {
            order_id: format!("paper-{}", state.trade_history.len()),
            status: OrderStatus::Filled,
            timestamp: Utc::now().timestamp_millis(),
        })
    }
}

#[async_trait]
impl ExecutionTarget for PaperBroker {
    async fn execute(&self, order: Order) -> Result<OrderResult, ExecutionError> {
        let mut state = self.state.lock().expect("paper broker mutex poisoned");
        self.execute_locked(&mut state, order)
    }

    async fn get_positions(&self) -> Result<Vec<Position>, ExecutionError> {
        let state = self.state.lock().expect("paper broker mutex poisoned");
        Ok(state
            .positions
            .values()
            .map(|position| Position {
                symbol: position.symbol.clone(),
                quantity: position.quantity,
                average_price: position.avg_entry_price,
                ltp: position.avg_entry_price,
                realized_pnl: 0.0,
                unrealized_pnl: position.unrealized_pnl,
            })
            .collect())
    }

    fn is_paper(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "paper"
    }
}

fn execute_buy(
    state: &mut PaperBrokerInner,
    symbol: &str,
    quantity: usize,
    price: f64,
) -> Result<(), ExecutionError> {
    let cost = quantity as f64 * price;
    if cost > state.cash {
        return Err(ExecutionError::insufficient_funds(
            "paper broker has insufficient cash",
        ));
    }

    state.cash -= cost;
    let position = state
        .positions
        .entry(symbol.to_string())
        .or_insert_with(|| PaperPosition {
            symbol: symbol.to_string(),
            quantity: 0,
            avg_entry_price: 0.0,
            unrealized_pnl: 0.0,
        });

    let previous_qty = position.quantity;
    let new_qty = previous_qty + quantity as i64;
    position.avg_entry_price = if new_qty == 0 {
        0.0
    } else {
        ((previous_qty as f64 * position.avg_entry_price) + (quantity as f64 * price))
            / new_qty as f64
    };
    position.quantity = new_qty;

    state.trade_history.push(PaperTrade {
        id: format!("paper-trade-{}", state.trade_history.len() + 1),
        timestamp: Utc::now().timestamp_millis(),
        symbol: symbol.to_string(),
        side: OrderSide::Buy,
        quantity,
        price,
        rule_id: String::new(),
        pnl: None,
    });

    Ok(())
}

fn execute_sell(
    state: &mut PaperBrokerInner,
    symbol: &str,
    quantity: usize,
    price: f64,
) -> Result<(), ExecutionError> {
    let position = state
        .positions
        .get_mut(symbol)
        .ok_or_else(|| ExecutionError::insufficient_position("no open paper position"))?;

    if position.quantity < quantity as i64 {
        return Err(ExecutionError::insufficient_position(
            "paper broker has insufficient position quantity",
        ));
    }

    let realized_pnl = (price - position.avg_entry_price) * quantity as f64;
    state.cash += quantity as f64 * price;
    position.quantity -= quantity as i64;

    state.trade_history.push(PaperTrade {
        id: format!("paper-trade-{}", state.trade_history.len() + 1),
        timestamp: Utc::now().timestamp_millis(),
        symbol: symbol.to_string(),
        side: OrderSide::Sell,
        quantity,
        price,
        rule_id: String::new(),
        pnl: Some(realized_pnl),
    });

    if position.quantity == 0 {
        state.positions.remove(symbol);
    }

    Ok(())
}

fn broker_state(state: &PaperBrokerInner) -> PaperBrokerState {
    PaperBrokerState {
        cash: state.cash,
        initial_cash: state.initial_cash,
        positions: state.positions.values().cloned().collect(),
        total_realized_pnl: state.trade_history.iter().filter_map(|trade| trade.pnl).sum(),
        trade_history: state.trade_history.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::OrderType;
    use crate::strategy::execution::target::ExecutionErrorKind;

    fn order(side: OrderSide, quantity: u32, price: f64) -> Order {
        Order {
            symbol: "NIFTY".to_string(),
            side,
            quantity,
            order_type: OrderType::Market,
            price: Some(price),
        }
    }

    #[tokio::test]
    async fn buy_deducts_cash() {
        let broker = PaperBroker::new("NIFTY".to_string(), 1_000.0);
        broker.execute(order(OrderSide::Buy, 2, 100.0)).await.unwrap();
        assert_eq!(broker.get_state().cash, 800.0);
    }

    #[tokio::test]
    async fn buy_updates_average_entry_price() {
        let broker = PaperBroker::new("NIFTY".to_string(), 10_000.0);
        broker.execute(order(OrderSide::Buy, 1, 100.0)).await.unwrap();
        broker.execute(order(OrderSide::Buy, 3, 200.0)).await.unwrap();
        let position = broker.get_position("NIFTY").unwrap();
        assert_eq!(position.quantity, 4);
        assert_eq!(position.avg_entry_price, 175.0);
    }

    #[tokio::test]
    async fn sell_credits_cash_and_records_pnl() {
        let broker = PaperBroker::new("NIFTY".to_string(), 10_000.0);
        broker.execute(order(OrderSide::Buy, 2, 100.0)).await.unwrap();
        broker.execute(order(OrderSide::Sell, 1, 125.0)).await.unwrap();
        let state = broker.get_state();
        assert_eq!(state.cash, 9_925.0);
        assert_eq!(state.total_realized_pnl, 25.0);
    }

    #[tokio::test]
    async fn sell_with_no_position_returns_insufficient_position() {
        let broker = PaperBroker::new("NIFTY".to_string(), 10_000.0);
        let err = broker
            .execute(order(OrderSide::Sell, 1, 100.0))
            .await
            .unwrap_err();
        assert!(matches!(err.kind, ExecutionErrorKind::InsufficientPosition));
    }

    #[tokio::test]
    async fn buy_with_insufficient_funds_returns_error() {
        let broker = PaperBroker::new("NIFTY".to_string(), 50.0);
        let err = broker
            .execute(order(OrderSide::Buy, 1, 100.0))
            .await
            .unwrap_err();
        assert!(matches!(err.kind, ExecutionErrorKind::InsufficientFunds));
    }

    #[tokio::test]
    async fn reset_restores_initial_state() {
        let broker = PaperBroker::new("NIFTY".to_string(), 1_000.0);
        broker.execute(order(OrderSide::Buy, 2, 100.0)).await.unwrap();
        broker.reset();
        let state = broker.get_state();
        assert_eq!(state.cash, 1_000.0);
        assert!(state.positions.is_empty());
        assert!(state.trade_history.is_empty());
    }
}
