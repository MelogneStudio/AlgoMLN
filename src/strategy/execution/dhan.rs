use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use parking_lot::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    broker::{
        dhan::{DhanClient, DhanError},
        BrokerClient,
    },
    broker::dhan::models::DhanFundsLimit,
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
/// How often the background task refreshes the available-funds cache. Funds
/// change more slowly than realized loss, so a 60s tick is plenty.
const FUNDS_REFRESH_INTERVAL_SECS: u64 = 60;
/// After this many consecutive refresh failures the cache is marked stale.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// Per-order realized-loss refresh throttle (audit W5). `execute_with_meta`
/// fires an HTTP `GET /positions` after every successful place; without a
/// throttle, a 5-rule strategy that fires 5 orders on the same candle pays
/// 5 GETs back-to-back. The session tick loop already coalesces one refresh
/// per candle boundary via the 10s background ticker; this constant caps
/// how often the per-order refresh can compound that cost. Picked > 5s so
/// a multi-rule candle still gets a single refresh, but a burst of orders
/// across candles does not pile up.
const PER_ORDER_REFRESH_MIN_INTERVAL_SECS: u64 = 5;
/// Hard cap on `available_cash` until the first successful `/funds/limit`
/// fetch lands. Defends against sizing orders against a `f64::MAX` while the
/// cache is still warming — see audit item C1. Conservative default for an
/// Indian retail equity account; the user can always deploy with
/// `QuantitySpec::Fixed` if they actually have more.
const DEFAULT_AVAILABLE_CASH_CAP: f64 = 1_000_000.0;

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

/// Abstraction over "fetch the funds limit" so `available_cash` can be
/// unit-tested without a live HTTP client. Production wires this to
/// `Arc<DhanClient>`; tests inject a mock.
#[async_trait]
trait FundsSource: Send + Sync + std::fmt::Debug {
    async fn fetch(&self) -> anyhow::Result<DhanFundsLimit>;
}

#[async_trait]
impl FundsSource for DhanClient {
    async fn fetch(&self) -> anyhow::Result<DhanFundsLimit> {
        self.get_funds_limit().await
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
    /// Funds source driving the available-cash refresh. Same object as
    /// `client` in production; a mock in tests.
    funds_source: Arc<dyn FundsSource>,
    /// Cached realized-loss magnitude in rupees, always non-negative. See
    /// [`ExecutionTarget::realized_loss`] on this type for the exact
    /// definition. Refreshed by a background task every
    /// `REFRESH_INTERVAL_SECS` and immediately after every successful order
    /// placement.
    realized_loss: Arc<RwLock<f64>>,
    /// Cached available cash in INR. Initialized to `DEFAULT_AVAILABLE_CASH_CAP`
    /// so order sizing is bounded on a cold cache, and replaced with the
    /// `/funds/limit` response as soon as the first successful refresh lands.
    /// Refreshed by a background task every `FUNDS_REFRESH_INTERVAL_SECS`.
    available_cash: Arc<RwLock<f64>>,
    /// True when either the realized-loss or available-cash cache has gone
    /// stale. The live session loop reads this via [`DhanBroker::is_stale`]
    /// and pauses rather than trusting a stale value.
    funds_stale: Arc<AtomicBool>,
    /// Consecutive `refresh_realized_loss` failures. Reset to 0 on any
    /// success; when it reaches `MAX_CONSECUTIVE_FAILURES` the cache is marked
    /// stale.
    consecutive_failures: Arc<AtomicU32>,
    /// Consecutive `refresh_funds` failures. Reset to 0 on any success; when
    /// it reaches `MAX_CONSECUTIVE_FAILURES` the funds cache is marked stale.
    funds_failures: Arc<AtomicU32>,
    /// True once the realized-loss cache is unreliable (too many consecutive
    /// refresh failures). The live session loop reads this via
    /// [`DhanBroker::is_stale`] and pauses rather than trusting a stale zero.
    stale: Arc<AtomicBool>,
    /// Wall-clock instant of the most recent successful refresh of either
    /// cache. `None` until at least one refresh has succeeded. Used by
    /// `time_since_last_success` so the resume IPC can show the user how
    /// stale the cache really is. Updated by [`refresh_once`] /
    /// [`refresh_funds_once`] on every success.
    last_success: Arc<RwLock<Option<Instant>>>,
    /// W5 (audit): wall-clock instant of the most recent **attempted**
    /// realized-loss refresh, success or not. The per-order refresh inside
    /// `execute_with_meta` short-circuits when this is younger than
    /// `PER_ORDER_REFRESH_MIN_INTERVAL_SECS`, so a burst of orders within
    /// one candle issues at most one HTTP `GET /positions`. The 10s
    /// background ticker still drives a refresh on every candle boundary.
    last_realized_loss_refresh: Arc<RwLock<Instant>>,
    /// Handle to the background refresh task, aborted on drop so tests (and
    /// short-lived sessions) do not leak tasks.
    refresh_task: Mutex<Option<JoinHandle<()>>>,
    /// Append-only trade log. Every successfully placed order is recorded here
    /// regardless of which code path invoked `execute` or `execute_with_meta`.
    pub trade_log: Arc<TradeLog>,
    /// Per-session strategy context (id, name, mode). Set by `LiveSession::start`,
    /// cleared on stop. `None` outside an active live session.
    pub session_context: Arc<RwLock<Option<SessionContext>>>,
    /// When true, BUY (entry) orders are suppressed in `execute_with_meta`.
    /// SELL orders (stop-loss, take-profit, risk-breach closes) always execute.
    /// Set by `LiveSession::pause`, cleared by `LiveSession::resume`.
    ///
    /// L2 (audit): the field is `pub(crate)` so only `LiveSession` (and
    /// anything else inside the `algomln` crate) can read or write it
    /// directly. External callers — including the Tauri binary — must
    /// go through the typed snapshot getter
    /// [`DhanBroker::paused_for_entries_snapshot`] and the gated setter
    /// [`DhanBroker::set_paused_for_entries`], which both refuse to
    /// operate when no `SessionContext` is set (i.e. no live session is
    /// in flight). The previous `pub` field was a one-line tripwire that
    /// any future `Arc<DhanBroker>` clone could have used to flip the
    /// flag without going through `LiveSession::pause`.
    pub(crate) paused_for_entries: Arc<AtomicBool>,
    /// H3 (audit): cancellation token shared with the owning `LiveSession`.
    /// `execute_with_meta` returns an error before any broker HTTP call
    /// when the token is cancelled, so a `stop()` that fires
    /// mid-`place_order` aborts within a few hundred ms instead of
    /// waiting for the broker's response and writing a phantom trade-log
    /// row. `LiveSession::start` wires its own `cancel` token via
    /// [`DhanBroker::set_cancel_token`]; `DhanBroker::new` defaults to a
    /// fresh token so standalone tests (and any caller that never sees
    /// a session) keep working. Stored behind a `parking_lot::RwLock`
    /// so the broker is shared through `Arc<DhanBroker>` without
    /// requiring exclusive ownership of the Arc.
    cancel_token: parking_lot::RwLock<CancellationToken>,
}

impl DhanBroker {
    pub fn new(client: Arc<DhanClient>, trade_log: Arc<TradeLog>) -> Self {
        let positions_source: Arc<dyn PositionsSource> = client.clone();
        let order_placer: Arc<dyn OrderPlacer> = client.clone();
        let funds_source: Arc<dyn FundsSource> = client.clone();
        Self::spawn_with_source(
            client,
            positions_source,
            order_placer,
            funds_source,
            trade_log,
        )
    }

    fn spawn_with_source(
        client: Arc<DhanClient>,
        positions_source: Arc<dyn PositionsSource>,
        order_placer: Arc<dyn OrderPlacer>,
        funds_source: Arc<dyn FundsSource>,
        trade_log: Arc<TradeLog>,
    ) -> Self {
        let realized_loss = Arc::new(RwLock::new(0.0));
        // Start at the hard cap, not zero, so a `PercentCapital` rule that
        // fires before the first `/funds/limit` refresh lands is sized against
        // a sane upper bound (1 lakh INR by default), not `f64::MAX`.
        let available_cash = Arc::new(RwLock::new(DEFAULT_AVAILABLE_CASH_CAP));
        let funds_stale = Arc::new(AtomicBool::new(false));
        let consecutive_failures = Arc::new(AtomicU32::new(0));
        let funds_failures = Arc::new(AtomicU32::new(0));
        let stale = Arc::new(AtomicBool::new(false));
        let paused_for_entries = Arc::new(AtomicBool::new(false));
        // No refresh has succeeded yet — drives the resume-during-stale error
        // message ("broker refresh has never succeeded") and gates the
        // optional error in `time_since_last_success`.
        let last_success: Arc<RwLock<Option<Instant>>> = Arc::new(RwLock::new(None));
        // W5: stamp the throttle instant far in the past so the first
        // post-`place_order` refresh actually fires (a fresh broker must
        // warm the cache immediately, not wait 5s).
        let last_realized_loss_refresh: Arc<RwLock<Instant>> =
            Arc::new(RwLock::new(Instant::now() - Duration::from_secs(
                PER_ORDER_REFRESH_MIN_INTERVAL_SECS + 1,
            )));

        // Spawn the periodic refresh only when a Tokio runtime is available.
        // Plain `#[test]` constructions (and any non-async caller) simply get
        // no background task; the cache stays at its initial values until an
        // explicit `refresh_realized_loss` / `refresh_funds`.
        //
        // Both refreshes share one ticker so the failure counters advance on
        // a single 10s beat. Funds have their own slower interval
        // (`FUNDS_REFRESH_INTERVAL_SECS`) — every 6th tick fires a funds
        // refresh instead of realized-loss.
        let refresh_task = match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                let positions_source = positions_source.clone();
                let funds_source = funds_source.clone();
                let realized_loss = realized_loss.clone();
                let available_cash = available_cash.clone();
                let consecutive_failures = consecutive_failures.clone();
                let funds_failures = funds_failures.clone();
                let stale = stale.clone();
                let funds_stale = funds_stale.clone();
                let last_success = last_success.clone();
                Some(handle.spawn(async move {
                    let mut ticker =
                        tokio::time::interval(Duration::from_secs(REFRESH_INTERVAL_SECS));
                    // Skip the immediate first tick so the cache can warm up
                    // out-of-band (the live session calls `refresh_*` after
                    // subscribing, but for safety we don't want a thundering
                    // herd at t=0).
                    ticker.tick().await;
                    let mut tick_count: u64 = 0;
                    loop {
                        ticker.tick().await;
                        tick_count += 1;
                        refresh_once(
                            &positions_source,
                            &realized_loss,
                            &consecutive_failures,
                            &stale,
                            &last_success,
                        )
                        .await;
                        // Funds refresh every 60s (= every 6th 10s tick).
                        if tick_count % (FUNDS_REFRESH_INTERVAL_SECS / REFRESH_INTERVAL_SECS)
                            == 0
                        {
                            refresh_funds_once(
                                &funds_source,
                                &available_cash,
                                &funds_failures,
                                &funds_stale,
                                &last_success,
                            )
                            .await;
                        }
                    }
                }))
            }
            Err(_) => None,
        };

        Self {
            client,
            positions_source,
            order_placer,
            funds_source,
            realized_loss,
            available_cash,
            funds_stale,
            consecutive_failures,
            funds_failures,
            stale,
            last_success,
            last_realized_loss_refresh,
            refresh_task: Mutex::new(refresh_task),
            trade_log,
            session_context: Arc::new(RwLock::new(None)),
            paused_for_entries,
            cancel_token: parking_lot::RwLock::new(CancellationToken::new()),
        }
    }

    /// Refresh the cached realized-loss magnitude from the positions source.
    /// Driven by the background task and also called immediately after every
    /// successful order placement. On failure the cache is left untouched
    /// (never reset to zero) and the failure counter advances toward
    /// staleness.
    ///
    /// W5 (audit): when called from `execute_with_meta` after a successful
    /// order, the call short-circuits if a refresh was attempted within the
    /// last `PER_ORDER_REFRESH_MIN_INTERVAL_SECS`. The 10s background
    /// ticker already coalesces a refresh on every candle boundary, so
    /// the per-order path is only useful for "many orders on the same
    /// candle" — and that case is precisely the one where the throttle
    /// saves N HTTP GETs. Tests that call `refresh_realized_loss`
    /// directly bypass the throttle (`force = true` overload below) so
    /// deterministic assertions still work.
    pub async fn refresh_realized_loss(&self) {
        // Throttle: skip when the previous attempt was recent. The first
        // call on a fresh broker always passes (the field was stamped
        // far in the past at construction time).
        {
            let last = *self.last_realized_loss_refresh.read();
            if last.elapsed() < Duration::from_secs(PER_ORDER_REFRESH_MIN_INTERVAL_SECS) {
                return;
            }
            *self.last_realized_loss_refresh.write() = Instant::now();
        }
        refresh_once(
            &self.positions_source,
            &self.realized_loss,
            &self.consecutive_failures,
            &self.stale,
            &self.last_success,
        )
        .await;
    }

    /// W5 (audit): bypass the per-order throttle. Used by the background
    /// ticker (which already runs on its own cadence) and by tests that
    /// need to assert a refresh happened synchronously.
    pub async fn refresh_realized_loss_unchecked(&self) {
        *self.last_realized_loss_refresh.write() = Instant::now();
        refresh_once(
            &self.positions_source,
            &self.realized_loss,
            &self.consecutive_failures,
            &self.stale,
            &self.last_success,
        )
        .await;
    }

    /// Refresh the cached available-balance value from the funds source.
    /// Driven by the background task. On failure the cache is left at its
    /// last good value (or `DEFAULT_AVAILABLE_CASH_CAP` on a cold cache) so
    /// order sizing never inflates past a sane bound.
    pub async fn refresh_funds(&self) {
        refresh_funds_once(
            &self.funds_source,
            &self.available_cash,
            &self.funds_failures,
            &self.funds_stale,
            &self.last_success,
        )
        .await;
    }

    /// True once either the realized-loss or available-cash cache is stale
    /// (too many consecutive refresh failures). The safety metrics must not
    /// be trusted as zeros while stale — the live session loop pauses
    /// instead. See `H2` in the live execution audit for the realized-loss
    /// case; the funds case applies the same policy because
    /// `available_cash` directly sizes `PercentCapital` orders.
    pub fn is_stale(&self) -> bool {
        self.stale.load(Ordering::Relaxed) || self.funds_stale.load(Ordering::Relaxed)
    }

    /// Wall-clock duration since the most recent successful refresh of
    /// either cache, or `None` if neither cache has ever refreshed. Drives
    /// the resume-during-stale error message (H2) so the user can see how
    /// long the broker has been unreachable instead of getting a bare
    /// "stale" string.
    pub fn time_since_last_success(&self) -> Option<Duration> {
        let guard = self.last_success.read();
        guard.as_ref().map(|instant| instant.elapsed())
    }

    /// Enable or disable entry-order suppression. Called by `LiveSession::pause`
    /// (set `true`) and `LiveSession::resume` (set `false`). When set, BUY orders
    /// are rejected before reaching the broker; SELL orders (SL, TP, risk-breach
    /// closes) always go through so open positions remain protected.
    ///
    /// L2 (audit): the setter is gated on a live `SessionContext` being set
    /// on the broker. If no session is active, the call is a silent no-op
    /// (returning `false`) — the broker must not flip the flag outside an
    /// active session because there is no caller that should be reaching
    /// for it. The previous `pub` setter could have been called with an
    /// `Arc<DhanBroker>` clone by anyone who could reach the field.
    /// Returns `true` when the flag was actually changed, `false` when the
    /// call was rejected.
    pub fn set_paused_for_entries(&self, paused: bool) -> bool {
        if self.session_context.read().is_none() {
            return false;
        }
        self.paused_for_entries.store(paused, Ordering::Relaxed);
        true
    }

    /// Snapshot of the entry-suppression flag. Read-only so the gated
    /// plugin execution API can re-run the pause gate without exposing
    /// the underlying `AtomicBool` (L2 in the live execution audit —
    /// `paused_for_entries` stays `pub(crate)` and the public surface
    /// is a typed setter plus a snapshot getter).
    pub fn paused_for_entries_snapshot(&self) -> bool {
        self.paused_for_entries.load(Ordering::Relaxed)
    }

    /// H3 (audit): wire the owning `LiveSession`'s cancellation token into
    /// the broker so a `stop()` mid-`place_order` aborts the call before
    /// the trade-log row is appended. `LiveSession::start` is the only
    /// expected caller; tests can ignore this and rely on the default
    /// fresh token from `DhanBroker::new`. Takes `&self` because the
    /// broker is `Arc`-shared and we don't want callers to have to
    /// reach inside the Arc to swap the token.
    pub fn set_cancel_token(&self, token: CancellationToken) {
        *self.cancel_token.write() = token;
    }

    /// H3 (audit): returns the currently-wired cancellation token so the
    /// caller can clone it (e.g. a future plugin execution API that
    /// threads its own tasks onto the same cancellation hook). Rarely
    /// used — most callers should construct the token and call
    /// `set_cancel_token` instead.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel_token.read().clone()
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
        // H3 (audit): check the cancellation token before any broker work.
        // The owning `LiveSession` cancels its token on `stop()`; doing this
        // check first guarantees we never reach the broker HTTP call after
        // a stop, and the trade-log append is never reached either (the
        // append is downstream of the broker call). Returning the same
        // `ExecutionError` shape as a regular broker failure keeps the
        // engine log consistent — `OrderFailed { rule_id, error }` still
        // surfaces the cancellation reason to the strategy log.
        if self.cancel_token.read().is_cancelled() {
            let msg = format!(
                "session cancelled before order for {} could be placed",
                order.symbol
            );
            return Err(ExecutionError {
                message: msg.clone(),
                kind: ExecutionErrorKind::BrokerError(msg),
            });
        }

        // When the session is paused, entry (BUY) orders are suppressed so the
        // user does not accidentally open new positions. SELL orders (stop-loss,
        // take-profit, risk-breach closes) always execute to protect open positions.
        if self.paused_for_entries.load(Ordering::Relaxed) && order.side == OrderSide::Buy {
            eprintln!(
                "[dhan_broker] session paused: skipping entry BUY order for {}",
                order.symbol
            );
            let msg = format!("session paused: entry order for {} suppressed", order.symbol);
            return Err(ExecutionError {
                message: msg.clone(),
                kind: ExecutionErrorKind::BrokerError(msg),
            });
        }

        let result = match self.order_placer.place(order.clone()).await {
            Ok(result) => result,
            Err(error) => {
                // Surface a single-line trail for the failed attempt so the
                // engine log can correlate the symbol/quantity with the
                // broker error (the trade log intentionally only records
                // fills — see H1 below).
                eprintln!("[dhan_broker] order placement failed for {}: {error}", order.symbol);
                return Err(map_place_order_error(error));
            }
        };

        // Refresh realized-loss cache immediately after placement.
        self.refresh_realized_loss().await;

        let classified = classify_result(result)?;

        // H1 (audit): only FILL results (Traded / Filled) belong in the trade
        // log — the wire shape represents a "trade". Non-terminal statuses
        // (Transit / Pending) and no-fills still carry an `order_id` but
        // writing a row with `price = 0.0` and `order_status = "Transit"`
        // pollutes the UI's trade log with phantom lines. Terminal non-fills
        // (Rejected / Cancelled / Expired) were already turned into an `Err`
        // by `classify_result` so they never reach here.
        if !classified.status.is_fill() {
            return Ok(classified);
        }

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
            price: order.price.unwrap_or(0.0),
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
    last_success: &Arc<RwLock<Option<Instant>>>,
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
            // H2 (audit): record the wall-clock instant of every successful
            // refresh so the resume IPC can show the user how stale the
            // cache has gone.
            *last_success.write() = Some(Instant::now());
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

/// Fetch the funds limit and update the available-cash cache. On success
/// resets the failure counter and clears staleness; on failure advances the
/// counter (toward `MAX_CONSECUTIVE_FAILURES`) and, past the threshold,
/// marks the funds cache stale. The cache is **never** reset on failure —
/// the last good value stays in place so order sizing stays bounded. A
/// negative or non-finite `available_balance` from the broker is treated
/// as zero (should never happen, but defends against garbage responses).
async fn refresh_funds_once(
    funds_source: &Arc<dyn FundsSource>,
    available_cash: &Arc<RwLock<f64>>,
    funds_failures: &Arc<AtomicU32>,
    funds_stale: &Arc<AtomicBool>,
    last_success: &Arc<RwLock<Option<Instant>>>,
) {
    match funds_source.fetch().await {
        Ok(limit) => {
            let value = if limit.available_balance.is_finite() && limit.available_balance >= 0.0
            {
                limit.available_balance
            } else {
                0.0
            };
            *available_cash.write() = value;
            funds_failures.store(0, Ordering::Relaxed);
            funds_stale.store(false, Ordering::Relaxed);
            // H2 (audit): same success timestamp as the realized-loss
            // path — either refresh counts as "broker reachable."
            *last_success.write() = Some(Instant::now());
        }
        Err(error) => {
            let failures = funds_failures.fetch_add(1, Ordering::Relaxed) + 1;
            eprintln!("dhan_broker: available_cash refresh failed: {error}");
            if failures >= MAX_CONSECUTIVE_FAILURES {
                funds_stale.store(true, Ordering::Relaxed);
                eprintln!(
                    "dhan_broker: available_cash stale after {failures} failures — session should pause"
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
        // Returns the cached value from `GET /funds/limit`, never `f64::MAX`.
        // On a cold cache (no refresh has succeeded yet) the cache holds
        // `DEFAULT_AVAILABLE_CASH_CAP` so `PercentCapital` orders are sized
        // against a sane upper bound rather than a u64-overflowing sentinel.
        // If the cache has gone stale (3+ consecutive failures), this still
        // returns the last good value — `is_stale()` is the signal callers
        // should consult before trusting this number.
        *self.available_cash.read()
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
    use std::time::{Duration, Instant};

    use async_trait::async_trait;
    use parking_lot::{Mutex, RwLock};

    use crate::{
        broker::dhan::{DhanAuth, DhanClient, DhanError},
        broker::dhan::models::DhanFundsLimit,
        live::trade_log::TradeLog,
        models::{Order, OrderResult, OrderSide, OrderStatus, OrderType, Position},
        strategy::execution::{ExecutionError, ExecutionErrorKind, ExecutionTarget},
    };

    use super::{
        classify_result, map_place_order_error, DhanBroker, FundsSource, OrderPlacer,
        PositionsSource, SessionContext, CancellationToken, DEFAULT_AVAILABLE_CASH_CAP,
        MAX_CONSECUTIVE_FAILURES,
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

    /// Mock `FundsSource` for tests. `fail=true` returns an error on every
    /// fetch; otherwise returns the configured `available_balance`.
    #[derive(Debug)]
    struct MockFunds {
        available_balance: f64,
        fail: bool,
    }

    #[async_trait]
    impl FundsSource for MockFunds {
        async fn fetch(&self) -> anyhow::Result<DhanFundsLimit> {
            if self.fail {
                anyhow::bail!("mock funds failure");
            }
            Ok(DhanFundsLimit {
                available_balance: self.available_balance,
                sod_limit: 0.0,
                collateral_amount: 0.0,
                receiveable_amount: 0.0,
                utilized_amount: 0.0,
                withdrawable_balance: 0.0,
                blocked_payout_amount: 0.0,
            })
        }
    }

    fn never_failing_funds() -> Arc<dyn FundsSource> {
        Arc::new(MockFunds {
            available_balance: 0.0,
            fail: false,
        })
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
    /// refresh task, so refresh timing in tests is fully explicit. Funds
    /// source defaults to a never-failing mock that returns 0.
    fn broker_with_source(source: Arc<dyn PositionsSource>) -> DhanBroker {
        let client = Arc::new(DhanClient::new(DhanAuth::new("test-token").unwrap()));
        let (log, _dir) = temp_trade_log();
        DhanBroker {
            client: client.clone(),
            positions_source: source,
            order_placer: client,
            funds_source: never_failing_funds(),
            realized_loss: Arc::new(RwLock::new(0.0)),
            available_cash: Arc::new(RwLock::new(DEFAULT_AVAILABLE_CASH_CAP)),
            funds_stale: Arc::new(AtomicBool::new(false)),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
            funds_failures: Arc::new(AtomicU32::new(0)),
            stale: Arc::new(AtomicBool::new(false)),
            last_success: Arc::new(RwLock::new(None)),
            // W5: stamp the throttle instant far in the past so tests
            // that call `refresh_realized_loss()` repeatedly see every
            // attempt fire (otherwise the staleness-after-N-failures test
            // would silently be a no-op).
            last_realized_loss_refresh: Arc::new(RwLock::new(
                Instant::now() - Duration::from_secs(PER_ORDER_REFRESH_MIN_INTERVAL_SECS + 1),
            )),
            refresh_task: Mutex::new(None),
            trade_log: log,
            session_context: Arc::new(RwLock::new(None)),
            paused_for_entries: Arc::new(AtomicBool::new(false)),
            cancel_token: parking_lot::RwLock::new(CancellationToken::new()),
        }
    }

    /// Build a broker with injectable positions, orders, and funds sources
    /// and a real temp-file `TradeLog`.
    fn broker_with_mocks(
        positions: Arc<dyn PositionsSource>,
        orders: Arc<dyn OrderPlacer>,
        funds: Arc<dyn FundsSource>,
        trade_log: Arc<TradeLog>,
    ) -> DhanBroker {
        let client = Arc::new(DhanClient::new(DhanAuth::new("test-token").unwrap()));
        DhanBroker {
            client,
            positions_source: positions,
            order_placer: orders,
            funds_source: funds,
            realized_loss: Arc::new(RwLock::new(0.0)),
            available_cash: Arc::new(RwLock::new(DEFAULT_AVAILABLE_CASH_CAP)),
            funds_stale: Arc::new(AtomicBool::new(false)),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
            funds_failures: Arc::new(AtomicU32::new(0)),
            stale: Arc::new(AtomicBool::new(false)),
            last_success: Arc::new(RwLock::new(None)),
            last_realized_loss_refresh: Arc::new(RwLock::new(
                Instant::now() - Duration::from_secs(PER_ORDER_REFRESH_MIN_INTERVAL_SECS + 1),
            )),
            refresh_task: Mutex::new(None),
            trade_log,
            session_context: Arc::new(RwLock::new(None)),
            paused_for_entries: Arc::new(AtomicBool::new(false)),
            cancel_token: parking_lot::RwLock::new(CancellationToken::new()),
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

    /// C1: On a cold cache, `available_cash` must NOT be `f64::MAX` — it must
    /// return the bounded default so `PercentCapital` orders cannot
    /// oversize to u64 overflow.
    #[test]
    fn test_available_cash_starts_at_bounded_default() {
        assert_eq!(broker().available_cash(), DEFAULT_AVAILABLE_CASH_CAP);
        assert!(broker().available_cash().is_finite());
    }

    /// C1: A successful `/funds/limit` refresh replaces the default with the
    /// reported balance.
    #[tokio::test]
    async fn test_available_cash_refreshes_from_funds_source() {
        let funds: Arc<dyn FundsSource> = Arc::new(MockFunds {
            available_balance: 250_000.0,
            fail: false,
        });
        let positions: Arc<dyn PositionsSource> = Arc::new(MockPositions {
            positions: Vec::new(),
            fail: false,
        });
        let (log, _dir) = temp_trade_log();
        let broker = DhanBroker {
            client: Arc::new(DhanClient::new(DhanAuth::new("test-token").unwrap())),
            positions_source: positions,
            order_placer: Arc::new(DhanClient::new(DhanAuth::new("test-token").unwrap())),
            funds_source: funds,
            realized_loss: Arc::new(RwLock::new(0.0)),
            available_cash: Arc::new(RwLock::new(DEFAULT_AVAILABLE_CASH_CAP)),
            funds_stale: Arc::new(AtomicBool::new(false)),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
            funds_failures: Arc::new(AtomicU32::new(0)),
            stale: Arc::new(AtomicBool::new(false)),
            last_success: Arc::new(RwLock::new(None)),
            last_realized_loss_refresh: Arc::new(RwLock::new(
                Instant::now() - Duration::from_secs(PER_ORDER_REFRESH_MIN_INTERVAL_SECS + 1),
            )),
            refresh_task: Mutex::new(None),
            trade_log: log,
            session_context: Arc::new(RwLock::new(None)),
            paused_for_entries: Arc::new(AtomicBool::new(false)),
            cancel_token: parking_lot::RwLock::new(CancellationToken::new()),
        };
        broker.refresh_funds().await;
        assert_eq!(broker.available_cash(), 250_000.0);
    }

    /// C1: A failed refresh leaves the cache at its last good value (does
    /// not reset to zero and does not poison with `f64::MAX`). Three
    /// consecutive failures mark the broker stale.
    #[tokio::test]
    async fn test_available_cash_stale_keeps_last_good_value() {
        let failing_funds: Arc<dyn FundsSource> = Arc::new(MockFunds {
            available_balance: 0.0,
            fail: true,
        });
        let positions: Arc<dyn PositionsSource> = Arc::new(MockPositions {
            positions: Vec::new(),
            fail: false,
        });
        let (log, _dir) = temp_trade_log();
        let broker = DhanBroker {
            client: Arc::new(DhanClient::new(DhanAuth::new("test-token").unwrap())),
            positions_source: positions,
            order_placer: Arc::new(DhanClient::new(DhanAuth::new("test-token").unwrap())),
            funds_source: failing_funds,
            realized_loss: Arc::new(RwLock::new(0.0)),
            available_cash: Arc::new(RwLock::new(42_000.0)),
            funds_stale: Arc::new(AtomicBool::new(false)),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
            funds_failures: Arc::new(AtomicU32::new(0)),
            stale: Arc::new(AtomicBool::new(false)),
            last_success: Arc::new(RwLock::new(None)),
            last_realized_loss_refresh: Arc::new(RwLock::new(
                Instant::now() - Duration::from_secs(PER_ORDER_REFRESH_MIN_INTERVAL_SECS + 1),
            )),
            refresh_task: Mutex::new(None),
            trade_log: log,
            session_context: Arc::new(RwLock::new(None)),
            paused_for_entries: Arc::new(AtomicBool::new(false)),
            cancel_token: parking_lot::RwLock::new(CancellationToken::new()),
        };
        assert!(!broker.is_stale());
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            broker.refresh_funds().await;
        }
        assert!(broker.is_stale());
        assert_eq!(
            broker.available_cash(),
            42_000.0,
            "stale funds must not reset to zero or inflate to f64::MAX"
        );
    }

    /// H2: `time_since_last_success` returns `None` before any refresh
    /// has succeeded; after a successful refresh it returns a duration
    /// since that instant. Drives the user-facing message that the resume
    /// IPC returns when the broker is stale.
    #[tokio::test]
    async fn test_time_since_last_success_none_before_refresh() {
        let broker = broker();
        assert!(
            broker.time_since_last_success().is_none(),
            "no refresh has happened yet"
        );
    }

    #[tokio::test]
    async fn test_time_since_last_success_after_refresh() {
        let source = Arc::new(MockPositions {
            positions: Vec::new(),
            fail: false,
        });
        let broker = broker_with_source(source);
        broker.refresh_realized_loss().await;
        let elapsed = broker
            .time_since_last_success()
            .expect("last_success must be set after a successful refresh");
        assert!(
            elapsed < Duration::from_secs(5),
            "elapsed must be near-zero, got {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_time_since_last_success_unchanged_on_failure() {
        // A failed refresh must NOT advance `last_success` — the field
        // tracks "last successful refresh," not "last attempted."
        let source = Arc::new(MockPositions {
            positions: Vec::new(),
            fail: true,
        });
        let broker = broker_with_source(source);
        assert!(broker.time_since_last_success().is_none());
        broker.refresh_realized_loss().await;
        assert!(
            broker.time_since_last_success().is_none(),
            "failed refresh must not stamp last_success"
        );
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
    async fn test_paused_suppresses_buy_orders() {
        let (log, _dir) = temp_trade_log();
        let positions = Arc::new(MockPositions { positions: Vec::new(), fail: false });
        let order_placer = Arc::new(MockOrderPlacer {
            result: OrderResult {
                order_id: "test-order".to_string(),
                status: OrderStatus::Traded,
                timestamp: 0,
                correlation_id: "algomln-test".to_string(),
            },
        });
        let broker = broker_with_mocks(positions, order_placer, never_failing_funds(), log);
        // L2 (audit): the setter is gated on a SessionContext. Stand one
        // up so the flag actually flips — this is what `LiveSession::start`
        // does in production before `pause`/`resume` ever runs.
        *broker.session_context.write() = Some(SessionContext {
            strategy_id: "strat-pause-test".to_string(),
            strategy_name: "Pause Test".to_string(),
            mode: "live".to_string(),
        });
        assert!(
            broker.set_paused_for_entries(true),
            "set_paused_for_entries must succeed when a SessionContext is set"
        );

        let buy_order = Order {
            symbol: "RELIANCE".to_string(),
            side: OrderSide::Buy,
            quantity: 1,
            order_type: OrderType::Market,
            price: None,
        };
        let result = broker.execute_with_meta(buy_order, "", "").await;
        assert!(result.is_err(), "BUY order must be suppressed when paused");
        assert!(result.unwrap_err().message.contains("session paused"));
    }

    #[tokio::test]
    async fn test_paused_allows_sell_orders() {
        let (log, _dir) = temp_trade_log();
        let positions = Arc::new(MockPositions { positions: Vec::new(), fail: false });
        let order_placer = Arc::new(MockOrderPlacer {
            result: OrderResult {
                order_id: "test-sell".to_string(),
                status: OrderStatus::Traded,
                timestamp: 0,
                correlation_id: "algomln-test".to_string(),
            },
        });
        let broker = broker_with_mocks(positions, order_placer, never_failing_funds(), log);
        *broker.session_context.write() = Some(SessionContext {
            strategy_id: "strat-pause-test".to_string(),
            strategy_name: "Pause Test".to_string(),
            mode: "live".to_string(),
        });
        broker.set_paused_for_entries(true);

        let sell_order = Order {
            symbol: "RELIANCE".to_string(),
            side: OrderSide::Sell,
            quantity: 1,
            order_type: OrderType::Market,
            price: None,
        };
        let result = broker.execute_with_meta(sell_order, "", "").await;
        assert!(result.is_ok(), "SELL order must go through when paused (exit orders always execute)");
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

        let broker = broker_with_mocks(
            positions_source,
            order_placer,
            never_failing_funds(),
            trade_log,
        );

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

    /// H3 (audit): a pre-cancelled `CancellationToken` must abort
    /// `execute_with_meta` before any broker call or trade-log append.
    /// This guards the safety net a `stop()` mid-`place_order` relies on:
    /// without this check, `execute_with_meta` would (a) issue an HTTP
    /// call after the user has been told the session is stopped and
    /// (b) write a phantom trade-log row for an order the user no
    /// longer consented to.
    #[tokio::test]
    async fn test_cancel_token_aborts_execute_with_meta() {
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("trade_log.jsonl");
        let trade_log = Arc::new(TradeLog::open(log_path.clone()).unwrap());

        let order_placer = Arc::new(MockOrderPlacer {
            result: OrderResult {
                order_id: "should-never-place".to_string(),
                status: OrderStatus::Traded,
                timestamp: 0,
                correlation_id: "algomln-cancel".to_string(),
            },
        });
        let positions = Arc::new(MockPositions {
            positions: Vec::new(),
            fail: false,
        });

        let mut broker = broker_with_mocks(
            positions,
            order_placer.clone(),
            never_failing_funds(),
            trade_log.clone(),
        );

        // Pre-cancel the token before the first order is ever placed.
        let token = CancellationToken::new();
        token.cancel();
        broker.set_cancel_token(token);
        // Note: `set_cancel_token(&self)` doesn't actually need `&mut`,
        // we just keep `mut broker` so `execute_with_meta` is callable
        // through the trait object (`Arc<dyn ExecutionTarget>::execute`
        // requires `&self` — we're calling through concrete type, fine).

        let order = Order {
            symbol: "NIFTY".to_string(),
            side: OrderSide::Buy,
            quantity: 1,
            order_type: OrderType::Market,
            price: Some(22000.0),
        };
        let err = broker
            .execute_with_meta(order, "rule-1", "")
            .await
            .expect_err("execute_with_meta must error on a pre-cancelled token");
        assert!(err.message.contains("session cancelled"), "got: {}", err.message);

        // The trade log must NOT carry a row for the cancelled attempt —
        // no place_order means no entry, by construction.
        let entries = TradeLog::read_all(&log_path).unwrap();
        assert!(
            entries.is_empty(),
            "cancelled order must not write to the trade log; got {:?}",
            entries
        );
    }

    /// H3 (audit): when the token is cancelled *after* the function has
    /// already returned (i.e. never observed), the broker behaves as
    /// before — the check is a snapshot, not a future. This guards the
    /// "session is stable, no cancel yet" path so we don't accidentally
    /// add an unexpected block.
    #[tokio::test]
    async fn test_cancel_token_does_not_block_when_unset() {
        let (log, _dir) = temp_trade_log();
        let positions = Arc::new(MockPositions {
            positions: Vec::new(),
            fail: false,
        });
        let order_placer = Arc::new(MockOrderPlacer {
            result: OrderResult {
                order_id: "ok".to_string(),
                status: OrderStatus::Traded,
                timestamp: 0,
                correlation_id: "algomln-fresh-token".to_string(),
            },
        });
        let broker = broker_with_mocks(positions, order_placer, never_failing_funds(), log);

        // Default token from spawn_with_source — never cancelled.
        let order = Order {
            symbol: "NIFTY".to_string(),
            side: OrderSide::Buy,
            quantity: 1,
            order_type: OrderType::Market,
            price: Some(22000.0),
        };
        let result = broker.execute_with_meta(order, "", "").await;
        assert!(result.is_ok(), "fresh token must not block");
    }

    /// L2 (audit): `set_paused_for_entries` must reject any call when no
    /// `SessionContext` is set on the broker. The flag stays at whatever
    /// value the broker was constructed with (default `false`), so a
    /// rogue `Arc<DhanBroker>` clone cannot pause a non-existent session.
    #[test]
    fn test_set_paused_for_entries_rejected_without_session_context() {
        let (log, _dir) = temp_trade_log();
        let positions = Arc::new(MockPositions { positions: Vec::new(), fail: false });
        let order_placer = Arc::new(MockOrderPlacer {
            result: OrderResult {
                order_id: "unused".to_string(),
                status: OrderStatus::Traded,
                timestamp: 0,
                correlation_id: "algomln-no-session".to_string(),
            },
        });
        let broker = broker_with_mocks(positions, order_placer, never_failing_funds(), log);
        assert!(broker.session_context.read().is_none());

        // No session context: setter returns false and the flag stays false.
        assert!(
            !broker.set_paused_for_entries(true),
            "setter must reject when no SessionContext is set"
        );
        assert!(
            !broker.paused_for_entries_snapshot(),
            "flag must not flip when the setter rejects the call"
        );
        assert!(
            !broker.set_paused_for_entries(false),
            "clear path must also be gated"
        );
    }

    /// W5 (audit): a `refresh_realized_loss` call within the throttle
    /// window must short-circuit so a burst of orders on the same candle
    /// does not pile up N HTTP `GET /positions` calls. We assert this
    /// with an `AtomicUsize` fetch counter on the mock positions source —
    /// the second call inside the window must NOT touch the source.
    #[tokio::test]
    async fn test_refresh_realized_loss_throttles_within_window() {
        use std::sync::atomic::{AtomicUsize, Ordering as AOrd};

        #[derive(Debug)]
        struct CountingPositions {
            positions: Vec<Position>,
            fetch_count: Arc<AtomicUsize>,
        }

        #[async_trait::async_trait]
        impl PositionsSource for CountingPositions {
            async fn fetch(&self) -> anyhow::Result<Vec<Position>> {
                self.fetch_count.fetch_add(1, AOrd::Relaxed);
                Ok(self.positions.clone())
            }
        }

        let fetch_count = Arc::new(AtomicUsize::new(0));
        let source: Arc<dyn PositionsSource> = Arc::new(CountingPositions {
            positions: vec![position_with_pnl(-100.0)],
            fetch_count: fetch_count.clone(),
        });
        let broker = broker_with_source(source);
        // Pre-stamp the throttle instant so the first call is throttled
        // (we want to assert the second-and-subsequent calls are skipped
        // when the window is still open).
        *broker.last_realized_loss_refresh.write() =
            Instant::now() - Duration::from_secs(1);

        // First call: within the throttle window, so no fetch.
        broker.refresh_realized_loss().await;
        assert_eq!(
            fetch_count.load(AOrd::Relaxed),
            0,
            "first call inside the throttle window must short-circuit"
        );

        // Stamp the throttle instant far in the past so the next call fires.
        *broker.last_realized_loss_refresh.write() =
            Instant::now() - Duration::from_secs(PER_ORDER_REFRESH_MIN_INTERVAL_SECS + 1);
        broker.refresh_realized_loss().await;
        assert_eq!(
            fetch_count.load(AOrd::Relaxed),
            1,
            "after the window expires, the next call must hit the source"
        );

        // And the very next call (still inside the new window) must
        // short-circuit again — the throttle does not "leak."
        broker.refresh_realized_loss().await;
        assert_eq!(
            fetch_count.load(AOrd::Relaxed),
            1,
            "second call inside the new window must short-circuit again"
        );

        // `refresh_realized_loss_unchecked` bypasses the throttle —
        // background ticker uses it for that reason.
        broker.refresh_realized_loss_unchecked().await;
        assert_eq!(
            fetch_count.load(AOrd::Relaxed),
            2,
            "unchecked refresh must always hit the source"
        );
    }
}
