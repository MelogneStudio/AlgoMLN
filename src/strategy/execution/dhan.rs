use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::{
    broker::{dhan::DhanClient, BrokerClient},
    models::{Order, OrderResult, Position},
};

use super::target::{ExecutionError, ExecutionErrorKind, ExecutionTarget};

#[derive(Debug)]
pub struct DhanBroker {
    client: Arc<DhanClient>,
    realized_loss: Mutex<f64>,
}

impl DhanBroker {
    pub fn new(client: Arc<DhanClient>) -> Self {
        Self {
            client,
            realized_loss: Mutex::new(0.0),
        }
    }
}

#[async_trait]
impl ExecutionTarget for DhanBroker {
    async fn execute(&self, order: Order) -> Result<OrderResult, ExecutionError> {
        let before_realized_pnl = total_realized_pnl(self.client.get_positions().await);
        let result = self.client.place_order(order).await.map_err(broker_error)?;

        let after_realized_pnl = total_realized_pnl(self.client.get_positions().await);
        if let (Some(before), Some(after)) = (before_realized_pnl, after_realized_pnl) {
            let realized_delta = after - before;
            if realized_delta < 0.0 {
                let mut loss = self
                    .realized_loss
                    .lock()
                    .expect("dhan broker mutex poisoned");
                *loss += -realized_delta;
            }
        }

        Ok(result)
    }

    async fn get_positions(&self) -> Result<Vec<Position>, ExecutionError> {
        self.client.get_positions().await.map_err(broker_error)
    }

    fn realized_loss(&self) -> f64 {
        *self
            .realized_loss
            .lock()
            .expect("dhan broker mutex poisoned")
    }

    fn available_cash(&self) -> f64 {
        f64::MAX
    }

    fn is_paper(&self) -> bool {
        false
    }

    fn name(&self) -> &str {
        "dhan"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn total_realized_pnl(result: anyhow::Result<Vec<Position>>) -> Option<f64> {
    match result {
        Ok(positions) => Some(positions.iter().map(|position| position.realized_pnl).sum()),
        Err(error) => {
            eprintln!("[DhanBroker] could not snapshot realized PnL: {error}");
            None
        }
    }
}

fn broker_error(error: anyhow::Error) -> ExecutionError {
    let message = error.to_string();
    ExecutionError {
        message: message.clone(),
        kind: ExecutionErrorKind::BrokerError(message),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        broker::dhan::{DhanAuth, DhanClient},
        strategy::execution::ExecutionTarget,
    };

    use super::DhanBroker;

    fn broker() -> DhanBroker {
        let auth = DhanAuth::new("test-token").unwrap();
        DhanBroker::new(Arc::new(DhanClient::new(auth)))
    }

    #[test]
    fn test_dhan_broker_is_not_paper() {
        assert!(!broker().is_paper());
    }

    #[test]
    fn test_dhan_broker_name() {
        assert_eq!(broker().name(), "dhan");
    }
}
