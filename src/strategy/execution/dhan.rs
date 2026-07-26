use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};
use std::time::Duration;

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::{
    broker::{
        dhan::{DhanClient, DhanError},
        BrokerClient,
    },
    live::trade_log::{TradeLog, TradeLogEntry},
    models::{Order, OrderResult, OrderSide, Position},
};

use super::target::{ExecutionError, ExecutionErrorKind, ExecutionTarget};

/// Immutable context set once per live session. Read by `execute_with_meta`
/// to populate the trade log entry.
#[derive(Debug, Clone)]
pub struct SessionContext {
    pub strategy_id: String,
    pub strategy_name: String,
    pub mode: String,
}

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

/// Abstraction over "place an order" so `execute_with_meta` can be
/// unit-tested without a live HTTP client. Production wires this to
/// `Arc<DhanClient>`; tests inject a mock.
#[async_trait]
trait OrderPlacer: Send + Sync + std::fmt::Debug {
    async fn place(&self, order: Order) -> anyhow::Result<OrderResult>;
}

#[async_trait]
impl OrderPlacer for DhanClient {
    async fn place(&self, order: Order) -> anyhow::Result<OrderResult> {
        self.place_order(order).await
    }
}

#[derive(Debug)]
pub struct DhanBroker {
    client: Arc<DhanClient>,
    /// Positions source driving the realized-loss refresh. Same object as
    /// `client` in production; a mock in tests.
    positions_source: Arc<dyn PositionsSource>,
    /// Order placer used by `execute_with_meta`. Same object as `client` in
    /// production; a mock in tests.
    order_placer: Arc<dyn OrderPlacer>,
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
    /// Append-only trade log. Every successfully placed order is recorded here
    /// regardless of which code path invoked `execute` or `execute_with_meta`.
    pub trade_log: Arc<TradeLog>,
    /// Per-session strategy context (id, name, mode). Set by `LiveSession::start`,
    /// cleared on stop. `None` outside an active live session.
    pub session_context: Arc<RwLock<Option<SessionContext>>>,
}

impl DhanBroker {
    pub fn new(client: Arc<DhanClient>, trade_log: Arc<TradeLog>) -> Self {
        let positions_source: Arc<dyn PositionsSource> = client.clone();
        let order_placer: Arc<dyn OrderPlacer> = client.clone();
        Self::spawn_with_source(client, positions_source, order_placer, trade_log)
    }

    fn spawn_with_source(
        client: Arc<DhanClient>,
        positions_source: Arc<dyn PositionsSource>,
        order_placer: Arc<dyn OrderPlacer>,
        trade_log: Arc<TradeLog>,
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
            order_placer,
            realized_loss,
            consecutive_failures,
            stale,
            refresh_task: Mutex::new(refresh_task),
            trade_log,
            session_context: Arc::new(RwLock::new(None)),
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

    /// Place an order and record it in the trade log with caller-supplied
    /// metadata. This is the primary execution path. The `ExecutionTarget::execute`
    /// trait method delegates here with empty `rule_id`/`notes` so the audit log
    /// is always written regardless of call path.
    ///
    /// If appending to the trade log fails, an error is printed but NOT returned —
    /// the order has already been placed and we must not falsely signal failure.
    pub async fn execute_with_meta(
        &self,
        order: Order,
        rule_id: &str,
        notes: &str,
    ) -> Result<OrderResult, ExecutionError> {
        let result = match self.order_placer.place(order.clone()).await {
            Ok(result) => result,
            Err(error) => return Err(map_place_order_error(error)),
        };

        // Refresh realized-loss cache immediately after placement.
        self.refresh_realized_loss().await;

        let classified = classify_result(result)?;

        // Build and append the trade log entry. Log failures must not surface
        // as order failures — the order is already through the broker.
        let ctx = self.session_context.read().clone();
        if ctx.is_none() {
            eprintln!(
                "[dhan_broker] execute_with_meta called with no active SessionContext — \
                 order placed but session metadata will be empty in the trade log"
            );
        }
        let (strategy_id, strategy_name, mode) = ctx
            .map(|c| (c.strategy_id, c.strategy_name, c.mode))
            .unwrap_or_else(|| (String::new(), String::new(), "live".to_string()));

        let price = if classified.status.is_fill() {
            order.price.unwrap_or(0.0)
        } else {
            0.0
        };

        let entry = TradeLogEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            strategy_id,
            strategy_name,
            symbol: order.symbol.clone(),
            side: match order.side {
                OrderSide::Buy => "BUY".to_string(),
                OrderSide::Sell => "SELL".to_string(),
            },
            quantity: i64::from(order.quantity),
            price,
            order_id: classified.order_id.clone(),
            order_status: format!("{:?}", classified.status),
            mode,
            rule_id: rule_id.to_string(),
            notes: notes.to_string(),
        };

        if let Err(io_err) = self.trade_log.append(entry) {
            eprintln!(
                "[dhan_broker] WARN: failed to append trade log for order {}: {io_err}",
                classified.order_id
            );
        }

        Ok(classified)
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
    /// Delegates to [`DhanBroker::execute_with_meta`] with empty rule/notes so
    /// every execution path — including plugins that call through the trait
    /// object — still writes to the trade log.
    async fn execute(&self, order: Order) -> Result<OrderResult, ExecutionError> {
        self.execute_with_meta(order, "", "").await
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
        live::trade_log::TradeLog,
        models::{Order, OrderResult, OrderSide, OrderStatus, OrderType, Position},
        strategy::execution::{ExecutionError, ExecutionErrorKind, ExecutionTarget},
    };

    use super::{
        classify_result, map_place_order_error, DhanBroker, OrderPlacer, PositionsSource,
        SessionContext, MAX_CONSECUTIVE_FAILURES,
    };

    fn temp_trade_log() -> (Arc<TradeLog>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("trade_log.jsonl");
        let log = Arc::new(TradeLog::open(path).unwrap());
        (log, dir)
    }

    fn broker() -> DhanBroker {
        let auth = DhanAuth::new("test-token").unwrap();
        let (log, _dir) = temp_trade_log();
        // _dir intentionally leaked: the TempDir must outlive the broker in these tests.
        // For simple property tests the log path doesn't matter.
        DhanBroker::new(Arc::new(DhanClient::new(auth)), log)
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

    /// Mock `OrderPlacer` that always returns a fixed `OrderResult`.
    #[derive(Debug)]
    struct MockOrderPlacer {
        result: OrderResult,
    }

    #[async_trait]
    impl OrderPlacer for MockOrderPlacer {
        async fn place(&self, _order: Order) -> anyhow::Result<OrderResult> {
            Ok(self.result.clone())
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
        let (log, _dir) = temp_trade_log();
        DhanBroker {
            client: client.clone(),
            positions_source: source,
            order_placer: client,
            realized_loss: Arc::new(RwLock::new(0.0)),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
            stale: Arc::new(AtomicBool::new(false)),
            refresh_task: Mutex::new(None),
            trade_log: log,
            session_context: Arc::new(RwLock::new(None)),
        }
    }

    /// Build a broker with injectable positions + order sources and a real
    /// temp-file `TradeLog`. Returns the `TempDir` guard so the caller keeps
    /// the directory alive.
    fn broker_with_mocks(
        positions: Arc<dyn PositionsSource>,
        orders: Arc<dyn OrderPlacer>,
        trade_log: Arc<TradeLog>,
    ) -> DhanBroker {
        let client = Arc::new(DhanClient::new(DhanAuth::new("test-token").unwrap()));
        DhanBroker {
            client,
            positions_source: positions,
            order_placer: orders,
            realized_loss: Arc::new(RwLock::new(0.0)),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
            stale: Arc::new(AtomicBool::new(false)),
            refresh_task: Mutex::new(None),
            trade_log,
            session_context: Arc::new(RwLock::new(None)),
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

    #[tokio::test]
    async fn test_execute_writes_trade_log() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("trade_log.jsonl");
        let trade_log = Arc::new(TradeLog::open(log_path.clone()).unwrap());

        let order_result = OrderResult {
            order_id: "order-traded-1".to_string(),
            status: OrderStatus::Traded,
            timestamp: 0,
            correlation_id: "algomln-test-trade".to_string(),
        };

        let positions_source = Arc::new(MockPositions {
            positions: Vec::new(),
            fail: false,
        });
        let order_placer = Arc::new(MockOrderPlacer {
            result: order_result,
        });

        let broker = broker_with_mocks(positions_source, order_placer, trade_log);

        // Set a SessionContext so the log entry carries real strategy metadata.
        *broker.session_context.write() = Some(SessionContext {
            strategy_id: "strat-42".to_string(),
            strategy_name: "Momentum Cross".to_string(),
            mode: "live".to_string(),
        });

        let order = Order {
            symbol: "NIFTY".to_string(),
            side: OrderSide::Buy,
            quantity: 5,
            order_type: OrderType::Market,
            price: Some(22500.0),
        };

        let result = broker
            .execute_with_meta(order, "rule-1", "stop_loss")
            .await
            .expect("execute_with_meta must succeed for a TRADED result");

        assert!(result.status.is_fill());

        // Read back and verify.
        let entries = TradeLog::read_all(&log_path).unwrap();
        assert_eq!(entries.len(), 1, "exactly one trade log entry expected");
        let entry = &entries[0];
        assert_eq!(entry.rule_id, "rule-1");
        assert_eq!(entry.notes, "stop_loss");
        assert_eq!(entry.strategy_id, "strat-42");
        assert_eq!(entry.strategy_name, "Momentum Cross");
        assert_eq!(entry.symbol, "NIFTY");
        assert_eq!(entry.side, "BUY");
        assert_eq!(entry.quantity, 5);
        assert_eq!(entry.price, 22500.0);
        assert_eq!(entry.order_id, "order-traded-1");
        assert_eq!(entry.order_status, "Traded");
        assert_eq!(entry.mode, "live");
        // id must be a non-empty uuid string
        assert!(!entry.id.is_empty());
    }
}
