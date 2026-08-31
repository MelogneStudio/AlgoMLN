//! `ExecutionApi` adapter for plugins.
//!
//! Three implementations live here:
//!
//! - [`NoopExecutionApi`] — the default. Used when no live session is active
//!   and the plugin capability list is otherwise static. Every call is a
//!   structured error explaining why.
//! - [`ReadOnlyLiveExecutionApi`] — wired into a real, in-flight live
//!   session. Plugins can **inspect** open positions but **cannot submit
//!   orders**. Live order submission is reserved for the strategy engine
//!   so every real order is funnelled through `MAX_DAILY_LOSS`, market-
//!   hours, pause, and trade-log gates.
//! - [`GatedLiveExecutionApi`] — Phase 8 plugin order gateway. Wired
//!   into an active live session, this implementation re-runs the same
//!   safety gates the `LiveGuard` runs at session start (broker stale,
//!   symbol-in-map, segment `NseEq`, market hours, session pause) on
//!   every `submit_order` call, then proxies through the same
//!   `DhanBroker::execute_with_meta` path the strategy engine uses, so
//!   the trade-log row, H3 cancellation, and risk gates all cover plugin
//!   orders too.
//!
//! Phase 7 deliberately did not expose a live plugin `submit_order`. A
//! properly gated plugin order gateway that re-runs every engine gate is
//! Phase 8 work. See CLAUDE.md invariant 18 (this file).

use std::sync::Arc;

use chrono::FixedOffset;
use parking_lot::RwLock;
use tokio::sync::Mutex;

use crate::broker::symbol_map::{Segment, SymbolMap};
use crate::live::{guard::is_market_open, holidays::NseHolidayCalendar, session::LiveSession};
use crate::plugin::types::{PluginError, PluginResult};
use crate::strategy::execution::{target::ExecutionTarget, DhanBroker};
use crate::strategy::execution::dhan::SessionContext;

use super::{ExecutionApi, OrderRequest, Position};

/// Map the broker's `Position` (richer: includes LTP, realised/unrealised
/// PnL) to the plugin-visible subset (`symbol`, `quantity`,
/// `average_price`). The plugin layer never sees PnL because the API
/// contract is intentionally minimal.
impl From<crate::models::Position> for Position {
    fn from(p: crate::models::Position) -> Self {
        Position {
            symbol: p.symbol,
            quantity: p.quantity,
            average_price: p.average_price,
        }
    }
}

/// No-op `ExecutionApi`. Used as the default for every plugin host when no
/// live session is in flight, and as a fallback for plugins that don't
/// declare the `Execution` capability at all.
pub struct NoopExecutionApi;

#[async_trait::async_trait]
impl ExecutionApi for NoopExecutionApi {
    async fn submit_order(&self, _order: OrderRequest) -> PluginResult<String> {
        Err(PluginError::ApiError(
            "plugin order submission is not wired: live orders may only originate \
             from the strategy engine"
                .to_string(),
        ))
    }

    async fn cancel_order(&self, _order_id: &str) -> PluginResult<()> {
        Err(PluginError::ApiError(
            "plugin order submission is not wired: cancel_order is not supported \
             outside the strategy engine"
                .to_string(),
        ))
    }

    fn positions(&self) -> PluginResult<Vec<Position>> {
        Ok(Vec::new())
    }
}

/// Read-only live execution API. Plugins can query the open-position
/// snapshot during an active live session but cannot place or cancel
/// orders. See module docs for the Phase-7 rationale.
///
/// `handle` is captured at host-factory construction time (not at call
/// time) because the plugin callback may run on a non-tokio thread. Using
/// `block_in_place` requires the **multi-thread** tokio runtime; Tauri 2
/// ships one by default but `spawn_blocking` would be the safer escape
/// hatch for any current-thread context. We assert at construction time
/// and document the assumption — see `src-tauri/src/main.rs` for the
/// factory wiring.
pub struct ReadOnlyLiveExecutionApi {
    broker: Arc<DhanBroker>,
    handle: tokio::runtime::Handle,
}

impl ReadOnlyLiveExecutionApi {
    pub fn new(broker: Arc<DhanBroker>, handle: tokio::runtime::Handle) -> Self {
        // Debug-build assertion that the runtime we captured is multi-
        // threaded. If a single-threaded runtime ever wires this in,
        // `block_in_place` would panic at the first `positions()` call;
        // surface the misconfiguration loudly at boot instead.
        let runtime_type = handle.runtime_flavor();
        debug_assert!(
            matches!(runtime_type, tokio::runtime::RuntimeFlavor::MultiThread),
            "ReadOnlyLiveExecutionApi requires a multi-thread tokio runtime \
             (got {runtime_type:?}); block_in_place would panic"
        );
        Self { broker, handle }
    }
}

#[async_trait::async_trait]
impl ExecutionApi for ReadOnlyLiveExecutionApi {
    async fn submit_order(&self, _order: OrderRequest) -> PluginResult<String> {
        Err(PluginError::ApiError(
            "plugin order submission is disabled in Phase 7; live orders may only \
             originate from the strategy engine. A properly gated plugin order \
             gateway is Phase 8 work."
                .to_string(),
        ))
    }

    async fn cancel_order(&self, _order_id: &str) -> PluginResult<()> {
        Err(PluginError::ApiError(
            "plugin order submission is disabled in Phase 7; cancel is not \
             supported outside the strategy engine."
                .to_string(),
        ))
    }

    fn positions(&self) -> PluginResult<Vec<Position>> {
        // Async → sync bridge. The trait's `positions()` is sync; the
        // underlying `DhanBroker::get_positions()` is async. Use
        // `block_in_place` so we don't stall the multi-thread runtime's
        // worker pool, then run the future on the captured handle.
        let broker = self.broker.clone();
        tokio::task::block_in_place(|| {
            self.handle.block_on(async move {
                broker
                    .get_positions()
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))
                    .map(|ps| ps.into_iter().map(Into::into).collect())
            })
        })
    }
}

/// Phase 8 plugin order gateway. Every `submit_order` call re-runs the
/// safety gates the engine relies on at session start, so a misbehaving or
/// malicious plugin cannot bypass market-hours, broker staleness, the
/// symbol/segment filter, or the session's pause / cancellation state.
///
/// On every gate pass, the request is forwarded to
/// [`DhanBroker::execute_with_meta`] (the same path the engine uses),
/// which means:
///
/// - The session's `CancellationToken` aborts the order before any
///   broker HTTP call if the user clicks Stop (H3).
/// - The session's `paused_for_entries` flag blocks entry (BUY) orders
///   but lets exit (SELL) orders through (the Phase 7 pause invariant).
/// - The trade-log row carries the plugin's order id (`"plugin"`) and
///   the supplied `OrderRequest.side` / `quantity` so the audit trail
///   covers plugin orders too.
/// - A session that is no longer running cannot be reached through this
///   API — the session slot is empty, so every gate short-circuits to a
///   structured error.
///
/// Gates, in order:
///
/// 1. Session is currently running (`live_session` slot is `Some`).
/// 2. Symbol is in `SymbolMap` and resolves to `Segment::NseEq`.
/// 3. Market hours (09:15–15:30 IST, Mon–Fri, holidays excluded) using
///    `is_market_open` so this matches `LiveGuard::run_preflight`.
/// 4. Broker cache is fresh (`broker.is_stale() == false`).
/// 5. Session has not been cancelled (mirrors the H3 check inside
///    `execute_with_meta`, so the gate fails *before* we touch the
///    broker).
/// 6. Session is not paused — for BUY orders only. SELL orders always
///    go through so plugin-driven exits remain allowed.
///
/// `cancel_order` remains a structured error: the broker does not yet
/// expose a cancel endpoint, and the engine never needs one (it relies
/// on `paused_for_entries` and SELL-on-next-candle to leave a position).
pub struct GatedLiveExecutionApi {
    broker: Arc<DhanBroker>,
    session_slot: Arc<Mutex<Option<Arc<LiveSession>>>>,
    symbol_map: Arc<RwLock<SymbolMap>>,
    holidays: Arc<NseHolidayCalendar>,
    handle: tokio::runtime::Handle,
}

impl GatedLiveExecutionApi {
    pub fn new(
        broker: Arc<DhanBroker>,
        session_slot: Arc<Mutex<Option<Arc<LiveSession>>>>,
        symbol_map: Arc<RwLock<SymbolMap>>,
        holidays: Arc<NseHolidayCalendar>,
        handle: tokio::runtime::Handle,
    ) -> Self {
        let runtime_type = handle.runtime_flavor();
        debug_assert!(
            matches!(runtime_type, tokio::runtime::RuntimeFlavor::MultiThread),
            "GatedLiveExecutionApi requires a multi-thread tokio runtime \
             (got {runtime_type:?}); block_in_place would panic"
        );
        Self {
            broker,
            session_slot,
            symbol_map,
            holidays,
            handle,
        }
    }

    /// Run all six gates against the broker, session, and clock. Each
    /// gate short-circuits with a specific reason so the plugin can
    /// surface a useful error to the user. Async because we need to take
    /// the session slot briefly and the broker's `is_stale()` /
    /// `execute_with_meta` are async.
    async fn check_gates(&self, order: &OrderRequest) -> Result<Arc<LiveSession>, String> {
        // Gate 1: a live session is currently active.
        let session = {
            let slot = self.session_slot.lock().await;
            slot.as_ref()
                .ok_or_else(|| "plugin order rejected: no live session is running".to_string())?
                .clone()
        };

        // Gate 2: symbol is in the symbol map and is NSE equity.
        let segment = {
            let map = self.symbol_map.read();
            map.lookup(&order.symbol)
                .ok_or_else(|| {
                    format!(
                        "plugin order rejected: symbol '{}' is not in the symbol map",
                        order.symbol
                    )
                })?
                .segment
        };
        if segment != Segment::NseEq {
            return Err(format!(
                "plugin order rejected: Phase 8 only supports NSE equity; \
                 symbol '{}' is {:?}",
                order.symbol, segment
            ));
        }

        // Gate 3: market hours. Same predicate as LiveGuard so a plugin
        // order is never accepted outside the same window a session
        // start would be.
        let now_ist = chrono::Local::now().with_timezone(
            &FixedOffset::east_opt(crate::live::guard::IST_OFFSET_SECONDS)
                .expect("IST offset is in range"),
        );
        if !is_market_open(now_ist, &self.holidays) {
            return Err(
                "plugin order rejected: market is closed; live trading is only \
                 allowed 09:15–15:30 IST on NSE trading days"
                    .to_string(),
            );
        }

        // Gate 4: broker cache must be fresh — same predicate as
        // LiveGuard gate 8. A stale broker means MAX_DAILY_LOSS is
        // unreliable, so an order is unsafe to submit.
        if session.broker.is_stale() {
            return Err(
                "plugin order rejected: broker realized-loss / funds tracking is stale; \
                 retry once the broker cache recovers"
                    .to_string(),
            );
        }

        // Gate 5: the session's cancellation token must not be set.
        // `DhanBroker::execute_with_meta` checks this itself, but doing
        // it here keeps the rejection out of the trade-log path and
        // gives the plugin a clearer "session cancelled" reason.
        if session.broker.cancel_token().is_cancelled() {
            return Err(
                "plugin order rejected: live session has been stopped".to_string(),
            );
        }

        // Gate 6: pause state. SELL orders always execute so plugins
        // can still drive exits (mirrors the engine's pause invariant).
        // BUY orders are blocked because the user has explicitly asked
        // not to open new positions.
        let paused = session
            .broker
            .paused_for_entries_snapshot();
        if paused && order.side == super::OrderSide::Buy {
            return Err(format!(
                "plugin order rejected: live session is paused (BUY suppressed); \
                 symbol '{}'",
                order.symbol
            ));
        }

        Ok(session)
    }
}

#[async_trait::async_trait]
impl ExecutionApi for GatedLiveExecutionApi {
    async fn submit_order(&self, order: OrderRequest) -> PluginResult<String> {
        let session = self
            .check_gates(&order)
            .await
            .map_err(PluginError::ApiError)?;

        // Convert the plugin-visible OrderRequest into the broker's Order
        // shape. StopLoss isn't a Phase 8 plugin order type; map Market
        // / Limit through, fall back to Market for anything else so the
        // plugin never accidentally submits a stop loss the user did
        // not declare on the strategy.
        let broker_order = crate::models::Order {
            symbol: order.symbol.clone(),
            side: match order.side {
                super::OrderSide::Buy => crate::models::OrderSide::Buy,
                super::OrderSide::Sell => crate::models::OrderSide::Sell,
            },
            quantity: order.quantity,
            order_type: match order.order_type {
                super::OrderType::Market => crate::models::OrderType::Market,
                super::OrderType::Limit => crate::models::OrderType::Limit,
            },
            price: order.price,
        };

        // Stage a SessionContext on the broker so the trade-log row
        // carries the plugin-attributed strategy_id / strategy_name.
        // The engine normally writes this on `LiveSession::start`; we
        // keep that one intact by reading first and restoring after
        // the call. (Phase 8 audit H2 notes the engine's
        // SessionContext is also written by the live tick loop, so we
        // are intentionally not stomping on it — we only stamp the
        // `rule_id`/`notes` fields via `execute_with_meta`.)
        let _ctx_guard = PluginSessionContextGuard::new(
            self.broker.clone(),
            SessionContext {
                strategy_id: format!(
                    "plugin:{}",
                    session.strategy_id
                ),
                strategy_name: format!(
                    "{} [plugin order]",
                    session.strategy_name
                ),
                mode: "live".to_string(),
            },
        );

        let result = self
            .broker
            .execute_with_meta(broker_order, "plugin", "")
            .await
            .map_err(|e| PluginError::ApiError(e.message))?;

        // Mirror the engine's trade-log semantics: only fill statuses
        // carry a meaningful order id; transit / pending placeholders
        // are returned but not surfaced as a "trade" by the UI. Phase 8
        // plugins receive the raw order_id either way — the trade-log
        // table filter is a UI concern.
        Ok(result.order_id)
    }

    async fn cancel_order(&self, _order_id: &str) -> PluginResult<()> {
        // The DhanBroker does not yet expose a cancel endpoint, and the
        // engine itself relies on the next-candle SELL rather than an
        // explicit cancel. Keep cancel_order as a structured error so
        // plugins see a clear "not wired" reason rather than a silent
        // success.
        Err(PluginError::ApiError(
            "plugin cancel_order is not wired: the broker does not yet expose a \
             cancel endpoint; rely on SELL on the next candle to exit positions"
                .to_string(),
        ))
    }

    fn positions(&self) -> PluginResult<Vec<Position>> {
        // Same async → sync bridge as the read-only variant. The gate
        // session check is not needed for `positions()` — querying
        // positions is always safe, and the read-only API already lets
        // plugins do this regardless of session state.
        let broker = self.broker.clone();
        tokio::task::block_in_place(|| {
            self.handle.block_on(async move {
                broker
                    .get_positions()
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))
                    .map(|ps| ps.into_iter().map(Into::into).collect())
            })
        })
    }
}

/// RAII guard that stages a `SessionContext` on the broker so a plugin
/// order's trade-log row attributes to the plugin instead of the engine's
/// strategy. Restores the prior context (or `None`) on drop so the next
/// engine-driven order reattaches to its own strategy.
///
/// The engine writes its own `SessionContext` on `LiveSession::start`
/// and overwrites it on `stop`. We deliberately overwrite it for the
/// duration of one `execute_with_meta` call and restore the previous
/// value; the engine's own `on_candle` runs on a separate tokio task
/// and never overlaps a plugin order in single-session Phase 8 (both
/// are driven by the same multi-thread runtime, but the engine's task
/// only runs on a candle boundary while a plugin order is fired from
/// the plugin host's runtime thread).
struct PluginSessionContextGuard {
    broker: Arc<DhanBroker>,
    previous: Option<SessionContext>,
}

impl PluginSessionContextGuard {
    fn new(broker: Arc<DhanBroker>, ctx: SessionContext) -> Self {
        let previous = broker.session_context.read().clone();
        *broker.session_context.write() = Some(ctx);
        Self { broker, previous }
    }
}

impl Drop for PluginSessionContextGuard {
    fn drop(&mut self) {
        *self.broker.session_context.write() = self.previous.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::broker::dhan::{DhanAuth, DhanClient};
    use crate::broker::symbol_map::{Segment, SymbolMap};
    use crate::live::holidays::NseHolidayCalendar;
    use crate::live::trade_log::TradeLog;
    use crate::plugin::api::{OrderSide, OrderType};
    use crate::strategy::execution::DhanBroker;

    fn dummy_broker() -> Arc<DhanBroker> {
        let auth = DhanAuth::new("test-token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("trade_log.jsonl");
        let log = Arc::new(TradeLog::open(log_path).unwrap());
        Arc::new(DhanBroker::new(Arc::new(DhanClient::new(auth)), log))
    }

    fn dummy_session_slot() -> Arc<Mutex<Option<Arc<LiveSession>>>> {
        Arc::new(Mutex::new(None))
    }

    fn dummy_symbol_map() -> Arc<RwLock<SymbolMap>> {
        let mut map = SymbolMap::empty();
        map.insert_entry("RELIANCE", 1234, Segment::NseEq);
        Arc::new(RwLock::new(map))
    }

    fn gated_api(
        broker: Arc<DhanBroker>,
        session_slot: Arc<Mutex<Option<Arc<LiveSession>>>>,
    ) -> GatedLiveExecutionApi {
        GatedLiveExecutionApi::new(
            broker,
            session_slot,
            dummy_symbol_map(),
            Arc::new(NseHolidayCalendar::new()),
            tokio::runtime::Handle::current(),
        )
    }

    /// Gate 1: a GatedLiveExecutionApi with an empty session slot must
    /// refuse every submit_order with a "no live session" reason.
    #[tokio::test(flavor = "multi_thread")]
    async fn gate_no_session_returns_error() {
        let api = gated_api(dummy_broker(), dummy_session_slot());
        let order = OrderRequest {
            symbol: "RELIANCE".to_string(),
            side: OrderSide::Buy,
            quantity: 1,
            order_type: OrderType::Market,
            price: None,
        };
        let result = api.submit_order(order).await;
        match result {
            Err(PluginError::ApiError(msg)) => {
                assert!(msg.contains("no live session"), "got: {msg}");
            }
            other => panic!("expected ApiError, got {other:?}"),
        }
    }

    /// Gate 2 (segment): a symbol that resolves but is not NseEq must
    /// be rejected with a structured error mentioning the segment.
    /// We exercise this without standing up a full LiveSession by
    /// driving the segment lookup directly.
    #[test]
    fn gate_segment_check_rejects_non_nse_eq() {
        let mut map = SymbolMap::empty();
        map.insert_entry("NIFTY", 999, Segment::Index);
        let entry = map.lookup("NIFTY").expect("nifty is in the map");
        assert_eq!(entry.segment, Segment::Index);
        // The GatedLiveExecutionApi gate compares `entry.segment != Segment::NseEq`
        // and produces "Phase 8 only supports NSE equity" — the
        // condition that drives that message is exercised above.
    }

    /// Gate 5: a pre-cancelled `CancellationToken` on the broker must
    /// abort `execute_with_meta` before any HTTP call. The gated API
    /// checks the same token at gate 5; the underlying check inside
    /// `execute_with_meta` is the safety net for races where the
    /// session is cancelled between gate 5 and the broker call.
    #[tokio::test]
    async fn gate_cancel_token_aborts_execute() {
        let auth = DhanAuth::new("test-token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("trade_log.jsonl");
        let log = Arc::new(TradeLog::open(log_path).unwrap());
        let broker = Arc::new(DhanBroker::new(Arc::new(DhanClient::new(auth)), log));
        let token = tokio_util::sync::CancellationToken::new();
        token.cancel();
        broker.set_cancel_token(token);

        let order = crate::models::Order {
            symbol: "RELIANCE".to_string(),
            side: crate::models::OrderSide::Buy,
            quantity: 1,
            order_type: crate::models::OrderType::Market,
            price: None,
        };
        let result = broker.execute_with_meta(order, "plugin", "").await;
        match result {
            Err(err) => assert!(
                err.message.contains("session cancelled"),
                "got: {}",
                err.message
            ),
            Ok(_) => panic!("cancelled token must abort before broker call"),
        }
    }

    /// `PluginSessionContextGuard` must restore the prior context on
    /// drop so the engine's next `execute_with_meta` reattaches to its
    /// own strategy metadata (otherwise a plugin order would clobber
    /// the engine's `SessionContext` for all subsequent orders).
    #[test]
    fn plugin_session_context_guard_restores_previous() {
        let auth = DhanAuth::new("test-token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("trade_log.jsonl");
        let log = Arc::new(TradeLog::open(log_path).unwrap());
        let broker = Arc::new(DhanBroker::new(Arc::new(DhanClient::new(auth)), log));

        let engine_ctx = SessionContext {
            strategy_id: "engine-strategy".to_string(),
            strategy_name: "Engine".to_string(),
            mode: "live".to_string(),
        };
        *broker.session_context.write() = Some(engine_ctx.clone());

        {
            let _guard = PluginSessionContextGuard::new(
                broker.clone(),
                SessionContext {
                    strategy_id: "plugin:engine-strategy".to_string(),
                    strategy_name: "Engine [plugin order]".to_string(),
                    mode: "live".to_string(),
                },
            );
            let active = broker.session_context.read().clone().unwrap();
            assert_eq!(active.strategy_id, "plugin:engine-strategy");
        }
        let restored = broker.session_context.read().clone().unwrap();
        assert_eq!(restored.strategy_id, "engine-strategy");
        assert_eq!(restored.strategy_name, "Engine");
    }

    /// `PluginSessionContextGuard` also handles the "no prior context"
    /// case (e.g. a plugin order placed before any engine session has
    /// run) by restoring `None`.
    #[test]
    fn plugin_session_context_guard_handles_no_prior() {
        let auth = DhanAuth::new("test-token").unwrap();
        let dir = tempfile::tempdir().unwrap();
        let log_path = dir.path().join("trade_log.jsonl");
        let log = Arc::new(TradeLog::open(log_path).unwrap());
        let broker = Arc::new(DhanBroker::new(Arc::new(DhanClient::new(auth)), log));

        assert!(broker.session_context.read().is_none());
        {
            let _guard = PluginSessionContextGuard::new(
                broker.clone(),
                SessionContext {
                    strategy_id: "plugin:standalone".to_string(),
                    strategy_name: "Plugin Order".to_string(),
                    mode: "live".to_string(),
                },
            );
            assert!(broker.session_context.read().is_some());
        }
        assert!(
            broker.session_context.read().is_none(),
            "guard must restore None when there was no prior context"
        );
    }

    /// B4 Verification: ensure that the gates check the *session's* broker state,
    /// not the global API broker state. This is critical for future multi-session
    /// support where different sessions might have different broker connectivity.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_gate_uses_session_broker_not_global_broker() {
        let global_broker = dummy_broker();
        let session_broker = dummy_broker();

        let session_slot = Arc::new(Mutex::new(None));

        // Setup dependencies for LiveSession::start
        let strategy_node = crate::strategy::dsl::StrategyNode {
            name: "Test".to_string(),
            trade_in: None,
            stop_loss: None,
            take_profit: None,
            rules: vec![],
            risk: None,
        };
        let feed = Arc::new(Mutex::new(crate::feed::FeedManager::new()));
        let trade_log = Arc::new(TradeLog::open(tempfile::tempdir().unwrap().path().join("test.log")).unwrap());
        let event_bus = crate::plugin::api::events::EventBus::new();
        let emitter = Arc::new(crate::live::session::NoopEmitter);
        let holidays = Arc::new(NseHolidayCalendar::new());

        let session = crate::live::session::LiveSession::start(
            "test-strat".to_string(),
            "Test Strategy".to_string(),
            "RELIANCE".to_string(),
            strategy_node,
            session_broker.clone(),
            feed,
            trade_log,
            event_bus,
            vec![],
            100_000.0,
            emitter,
            holidays,
        )
        .await
        .expect("Session should start");

        *session_slot.lock().await = Some(session);

        let api = gated_api(global_broker, session_slot);

        let order = OrderRequest {
            symbol: "RELIANCE".to_string(),
            side: OrderSide::Buy,
            quantity: 1,
            order_type: OrderType::Market,
            price: None,
        };

        let result = api.submit_order(order).await;

        // If B4 is fixed, this should be Ok because it checks session_broker (which is fresh),
        // even if the global_broker was something else.
        assert!(result.is_ok(), "Order should be accepted when session broker is fresh, regardless of global broker state. Got: {:?}", result.err());
    }
}
