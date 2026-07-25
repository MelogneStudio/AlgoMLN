use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::{
    broker::{
        dhan::{DhanClient, DhanError},
        BrokerClient,
    },
    models::{Order, OrderResult, Position},
};

use super::target::{ExecutionError, ExecutionErrorKind, ExecutionTarget};

/// How often the background task refreshes the realized-loss cache.
const REFRESH_INTERVAL_SECS: u64 = 10;
/// After this many consecutive refresh failures the cache is marked stale.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;

/// Abstraction over "fetch current positions" so the realized-loss refresh can
/// be unit-tested without a live HTTP client. Production wires this to
/// `Arc<DhanClient>`; tests inject a mock.
#[async_trait]
trait PositionsSource: Send + Sync + std::fmt::Debug {
    async fn fetch(&self) -> anyhow::Result<Vec<Position>>;
}

#[async_trait]
impl PositionsSource for DhanClient {
    async fn fetch(&self) -> anyhow::Result<Vec<Position>> {
        self.get_positions().await
    }
}

#[derive(Debug)]
pub struct DhanBroker {
    client: Arc<DhanClient>,
    /// Positions source driving the realized-loss refresh. Same object as
    /// `client` in production; a mock in tests.
    positions_source: Arc<dyn PositionsSource>,
    /// Cached realized-loss magnitude in rupees, always non-negative. See
    /// [`ExecutionTarget::realized_loss`] on this type for the exact
    /// definition. Refreshed by a background task every
    /// `REFRESH_INTERVAL_SECS` and immediately after every successful order
    /// placement.
    realized_loss: Arc<RwLock<f64>>,
    /// Consecutive `refresh_realized_loss` failures. Reset to 0 on any
    /// success; when it reaches `MAX_CONSECUTIVE_FAILURES` the cache is marked
    /// stale.
    consecutive_failures: Arc<AtomicU32>,
    /// True once the realized-loss cache is unreliable (too many consecutive
    /// refresh failures). The live session loop reads this via
    /// [`DhanBroker::is_stale`] and pauses rather than trusting a stale zero.
    stale: Arc<AtomicBool>,
    /// Handle to the background refresh task, aborted on drop so tests (and
    /// short-lived sessions) do not leak tasks.
    refresh_task: Mutex<Option<JoinHandle<()>>>,
}

impl DhanBroker {
    pub fn new(client: Arc<DhanClient>) -> Self {
        let positions_source: Arc<dyn PositionsSource> = client.clone();
        Self::spawn_with_source(client, positions_source)
    }

    fn spawn_with_source(
        client: Arc<DhanClient>,
        positions_source: Arc<dyn PositionsSource>,
    ) -> Self {
        let realized_loss = Arc::new(RwLock::new(0.0));
        let consecutive_failures = Arc::new(AtomicU32::new(0));
        let stale = Arc::new(AtomicBool::new(false));

        // Spawn the periodic refresh only when a Tokio runtime is available.
        // Plain `#[test]` constructions (and any non-async caller) simply get
        // no background task; the cache stays at its initial 0.0 until an
        // explicit `refresh_realized_loss`.
        let refresh_task = match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let positions_source = positions_source.clone();
                let realized_loss = realized_loss.clone();
                let consecutive_failures = consecutive_failures.clone();
                let stale = stale.clone();
                Some(handle.spawn(async move {
                    let mut ticker =
                        tokio::time::interval(Duration::from_secs(REFRESH_INTERVAL_SECS));
                    loop {
                        ticker.tick().await;
                        refresh_once(
                            &positions_source,
                            &realized_loss,
                            &consecutive_failures,
                            &stale,
                        )
                        .await;
                    }
                }))
            }
            Err(_) => None,
        };

        Self {
            client,
            positions_source,
            realized_loss,
            consecutive_failures,
            stale,
            refresh_task: Mutex::new(refresh_task),
        }
    }

    /// Refresh the cached realized-loss magnitude from the positions source.
    /// Driven by the background task and also called immediately after every
    /// successful order placement. On failure the cache is left untouched
    /// (never reset to zero) and the failure counter advances toward
    /// staleness.
    pub async fn refresh_realized_loss(&self) {
        refresh_once(
            &self.positions_source,
            &self.realized_loss,
            &self.consecutive_failures,
            &self.stale,
        )
        .await;
    }

    /// True once the realized-loss cache is stale (too many consecutive
    /// refresh failures). The safety metric must not be trusted as a zero
    /// while stale — the live session loop pauses instead.
    pub fn is_stale(&self) -> bool {
        self.stale.load(Ordering::Relaxed)
    }
}

impl Drop for DhanBroker {
    fn drop(&mut self) {
        if let Some(task) = self.refresh_task.lock().take() {
            task.abort();
        }
    }
}

/// Compute the realized-loss magnitude from a positions snapshot and update the
/// shared cache. On success resets the failure counter and clears staleness; on
/// failure advances the counter and, past the threshold, marks the cache stale.
async fn refresh_once(
    positions_source: &Arc<dyn PositionsSource>,
    realized_loss: &Arc<RwLock<f64>>,
    consecutive_failures: &Arc<AtomicU32>,
    stale: &Arc<AtomicBool>,
) {
    match positions_source.fetch().await {
        Ok(positions) => {
            // Sum the magnitude of every losing position; winning positions
            // contribute nothing. Profits never reduce the loss figure.
            let magnitude: f64 = positions
                .iter()
                .map(|position| position.realized_pnl)
                .filter(|pnl| *pnl < 0.0)
                .map(|pnl| -pnl)
                .sum();
            *realized_loss.write() = magnitude;
            consecutive_failures.store(0, Ordering::Relaxed);
            stale.store(false, Ordering::Relaxed);
        }
        Err(error) => {
            let failures = consecutive_failures.fetch_add(1, Ordering::Relaxed) + 1;
            eprintln!("dhan_broker: realized_loss refresh failed: {error}");
            if failures >= MAX_CONSECUTIVE_FAILURES {
                stale.store(true, Ordering::Relaxed);
                eprintln!(
                    "dhan_broker: realized_loss stale after {failures} failures — session should pause"
                );
            }
        }
    }
}

#[async_trait]
impl ExecutionTarget for DhanBroker {
    async fn execute(&self, order: Order) -> Result<OrderResult, ExecutionError> {
        let result = match self.client.place_order(order).await {
            Ok(result) => result,
            Err(error) => return Err(map_place_order_error(error)),
        };

        // Refresh the realized-loss cache immediately so the risk gate sees the
        // freshest number the broker can give us; the periodic task keeps it
        // current between orders.
        self.refresh_realized_loss().await;

        classify_result(result)
    }

    async fn get_positions(&self) -> Result<Vec<Position>, ExecutionError> {
        self.client.get_positions().await.map_err(broker_error)
    }

    /// Total realized loss for the session as a **non-negative magnitude** in
    /// rupees. Computed as the sum of `max(0, -realizedProfit)` across every
    /// position returned by Dhan's `GET /positions`: each losing position adds
    /// its absolute loss, winning positions add nothing, and profits never
    /// reduce the figure. Backed by a polling cache — a failed refresh leaves
    /// the last good value in place rather than reporting a falsely low zero,
    /// so callers feeding the `RISK MAX_DAILY_LOSS` gate should also consult
    /// [`DhanBroker::is_stale`].
    fn realized_loss(&self) -> f64 {
        *self.realized_loss.read()
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

fn broker_error(error: anyhow::Error) -> ExecutionError {
    let message = error.to_string();
    ExecutionError {
        message: message.clone(),
        kind: ExecutionErrorKind::BrokerError(message),
    }
}

/// Decide how a placed order's status maps to the `ExecutionTarget::execute`
/// contract. A terminal non-fill (rejected / cancelled / expired) is a hard
/// error. Non-terminal statuses (transit / pending) and fills both return `Ok`
/// with the status intact — the caller inspects `status.is_fill()` before
/// treating the result as an execution.
fn classify_result(result: OrderResult) -> Result<OrderResult, ExecutionError> {
    if result.status.is_terminal() && !result.status.is_fill() {
        let message = format!("order {} status: {:?}", result.order_id, result.status);
        return Err(ExecutionError {
            message: message.clone(),
            kind: ExecutionErrorKind::BrokerError(message),
        });
    }
    Ok(result)
}

/// Map a `place_order` error to an `ExecutionError`. A timed-out order surfaces
/// as [`DhanError::OrderStatusUnknown`]; we preserve its "check broker app"
/// message so the caller/UI never mistakes an unknown-status order for a plain
/// failure (or, worse, a fill). All other errors pass through `broker_error`.
fn map_place_order_error(error: anyhow::Error) -> ExecutionError {
    if let Some(DhanError::OrderStatusUnknown { correlation_id }) =
        error.downcast_ref::<DhanError>()
    {
        let message =
            format!("order status unknown, check broker app; correlation_id={correlation_id}");
        return ExecutionError {
            message: message.clone(),
            kind: ExecutionErrorKind::BrokerError(message),
        };
    }
    broker_error(error)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicBool, AtomicU32},
        Arc,
    };

    use async_trait::async_trait;
    use parking_lot::{Mutex, RwLock};

    use crate::{
        broker::dhan::{DhanAuth, DhanClient, DhanError},
        models::{OrderResult, OrderStatus, Position},
        strategy::execution::{ExecutionError, ExecutionErrorKind, ExecutionTarget},
    };

    use super::{
        classify_result, map_place_order_error, DhanBroker, PositionsSource,
        MAX_CONSECUTIVE_FAILURES,
    };

    fn broker() -> DhanBroker {
        let auth = DhanAuth::new("test-token").unwrap();
        DhanBroker::new(Arc::new(DhanClient::new(auth)))
    }

    #[derive(Debug)]
    struct MockPositions {
        positions: Vec<Position>,
        fail: bool,
    }

    #[async_trait]
    impl PositionsSource for MockPositions {
        async fn fetch(&self) -> anyhow::Result<Vec<Position>> {
            if self.fail {
                anyhow::bail!("mock positions failure");
            }
            Ok(self.positions.clone())
        }
    }

    fn position_with_pnl(realized_pnl: f64) -> Position {
        Position {
            symbol: "TEST".to_string(),
            quantity: 0,
            average_price: 0.0,
            ltp: 0.0,
            realized_pnl,
            unrealized_pnl: 0.0,
        }
    }

    /// Build a broker wired to a mock positions source with NO background
    /// refresh task, so refresh timing in tests is fully explicit.
    fn broker_with_source(source: Arc<dyn PositionsSource>) -> DhanBroker {
        let client = Arc::new(DhanClient::new(DhanAuth::new("test-token").unwrap()));
        DhanBroker {
            client,
            positions_source: source,
            realized_loss: Arc::new(RwLock::new(0.0)),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
            stale: Arc::new(AtomicBool::new(false)),
            refresh_task: Mutex::new(None),
        }
    }

    #[test]
    fn test_dhan_broker_is_not_paper() {
        assert!(!broker().is_paper());
    }

    #[test]
    fn test_dhan_broker_name() {
        assert_eq!(broker().name(), "dhan");
    }

    #[test]
    fn test_realized_loss_starts_at_zero() {
        assert_eq!(broker().realized_loss(), 0.0);
    }

    #[tokio::test]
    async fn test_realized_loss_magnitude_from_positions() {
        let source = Arc::new(MockPositions {
            positions: vec![position_with_pnl(-500.0), position_with_pnl(200.0)],
            fail: false,
        });
        let broker = broker_with_source(source);
        broker.refresh_realized_loss().await;
        // Losing position contributes 500; the +200 profit contributes 0.
        assert_eq!(broker.realized_loss(), 500.0);
    }

    #[tokio::test]
    async fn test_realized_loss_is_stale_after_failures() {
        let source = Arc::new(MockPositions {
            positions: Vec::new(),
            fail: true,
        });
        let broker = broker_with_source(source);
        assert!(!broker.is_stale());
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            broker.refresh_realized_loss().await;
        }
        assert!(broker.is_stale());
    }

    fn order_result(status: OrderStatus) -> OrderResult {
        OrderResult {
            order_id: "order-1".to_string(),
            status,
            timestamp: 0,
            correlation_id: "algomln-test".to_string(),
        }
    }

    #[test]
    fn test_rejected_returns_err() {
        let classified = classify_result(order_result(OrderStatus::Rejected));
        assert!(matches!(
            classified,
            Err(ExecutionError {
                kind: ExecutionErrorKind::BrokerError(_),
                ..
            })
        ));
    }

    #[test]
    fn test_transit_returns_ok_without_fill() {
        // Non-terminal statuses pass through as Ok with the status intact so
        // the caller can branch on `is_fill()` — they must NOT be errors.
        let classified = classify_result(order_result(OrderStatus::Transit))
            .expect("transit must not be an execution error");
        assert_eq!(classified.status, OrderStatus::Transit);
        assert!(!classified.status.is_fill());
    }

    #[test]
    fn test_traded_returns_ok() {
        let classified = classify_result(order_result(OrderStatus::Traded))
            .expect("traded must be Ok");
        assert!(classified.status.is_fill());
    }

    #[test]
    fn test_order_status_unknown_maps_to_broker_error() {
        let mapped = map_place_order_error(
            DhanError::OrderStatusUnknown {
                correlation_id: "algomln-abc".to_string(),
            }
            .into(),
        );
        assert!(matches!(
            mapped.kind,
            ExecutionErrorKind::BrokerError(_)
        ));
        assert!(mapped.message.contains("check broker app"));
        assert!(mapped.message.contains("algomln-abc"));
    }
}
