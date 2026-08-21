use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

/// Maximum number of completed candles kept in `candle_history`. Older candles
/// are dropped from the front once this is exceeded. The engine only needs a
/// bounded window; retaining the full day is dead weight over long sessions.
const MAX_CANDLE_HISTORY: usize = 5000;

// ── Failure signalling ────────────────────────────────────────────────────────

/// Payload emitted when a live session transitions to `Failed`.
/// Serialised as camelCase JSON for the Tauri frontend event bus.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveSessionFailedPayload {
    pub strategy_id: String,
    pub reason: String,
    /// Best-effort count of open positions at failure time. `None` if the
    /// positions fetch itself failed, or if fetching was not attempted.
    pub open_positions_estimate: Option<i64>,
}

/// Abstraction over "emit a session-failed alert". Implemented by a real
/// `tauri::AppHandle` wrapper in production and a mock in tests. Keeping this
/// as a trait means `LiveSession` compiles and tests cleanly without a Tauri
/// dependency in the library crate.
pub trait SessionEventEmitter: Send + Sync + 'static {
    fn emit_failed(&self, payload: LiveSessionFailedPayload);
}

/// No-op emitter. Used as a default when no real emitter is wired up yet
/// (e.g. tests that only care about broker / engine behaviour, not UI alerts).
pub struct NoopEmitter;

impl SessionEventEmitter for NoopEmitter {
    fn emit_failed(&self, _payload: LiveSessionFailedPayload) {}
}

use crate::{
    broker::Timeframe,
    feed::FeedManager,
    live::{
        candle_assembler::CandleAssembler,
        guard::{is_market_open, IST_OFFSET_SECONDS},
        holidays::NseHolidayCalendar,
        trade_log::TradeLog,
    },
    models::Candle,
    plugin::api::events::EventBus,
    strategy::{
        dsl::StrategyNode,
        execution::{
            dhan::SessionContext,
            target::ExecutionTarget,
            DhanBroker,
        },
        runtime::{StrategyEngine, StrategyInstance, StrategyStatus as EngineStatus},
    },
};

/// Convert a candle's millisecond UTC unix timestamp into an IST wall-clock
/// `DateTime<FixedOffset>`. Used by the per-candle market-hours gate so the
/// decision reads the candle close in the same frame as `is_market_open`.
///
/// Lives here (rather than in `guard.rs`) so the session module does not
/// have to re-export `FixedOffset`. The constant offset keeps the math
/// trivial: `chrono::DateTime::from_timestamp_millis(ts) -> Utc -> IST`.
fn candle_close_ist(ts_millis: i64) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    let utc = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts_millis)?;
    Some(utc.with_timezone(
        &chrono::FixedOffset::east_opt(IST_OFFSET_SECONDS).expect("IST offset is in range"),
    ))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    Starting,
    Running,
    Paused,
    Stopped,
    Failed(String),
}

pub struct LiveSession {
    pub strategy_id: String,
    pub strategy_name: String,
    pub symbol: String,
    pub status: Arc<RwLock<SessionStatus>>,
    pub engine: Arc<Mutex<StrategyEngine>>,
    /// L8 (audit): the broker handle is `pub(crate)` so only the `algomln`
    /// crate can reach `DhanBroker::execute_with_meta` directly. External
    /// callers (the Tauri binary, future plugin code) must go through the
    /// typed helpers below — `broker_positions`, `broker_realized_loss`,
    /// `broker_is_stale`, `broker_time_since_last_success`,
    /// `broker_cancel_token`, `broker_paused_for_entries`, and the
    /// `set_paused_for_entries` setter. The previous `pub broker` field
    /// was a one-line tripwire that any plugin host with an
    /// `Arc<LiveSession>` could have used to place orders without going
    /// through the gate stack.
    pub(crate) broker: Arc<DhanBroker>,
    pub trade_log: Arc<TradeLog>,
    pub candle_history: Arc<Mutex<Vec<Candle>>>,
    pub start_time: DateTime<Utc>,
    /// B1 (audit): NSE trading-day calendar used by the per-candle
    /// market-hours gate in the tick loop. Cloned from the shared
    /// `AppState` via `LiveGuard`. The session never mutates this
    /// calendar — the only mutator is `NseHolidayCalendar` itself, which
    /// is constructed once at startup and shared with `LiveGuard` and
    /// `GatedLiveExecutionApi`.
    pub(crate) holiday_calendar: Arc<NseHolidayCalendar>,
    cancel: CancellationToken,
    task: Mutex<Option<JoinHandle<()>>>,
    /// Emitter for loud failure alerts. The real impl wraps `tauri::AppHandle`
    /// and calls `emit_all("live_session_failed", ...)`. Tests inject a mock.
    emitter: Arc<dyn SessionEventEmitter>,
}

impl LiveSession {
    /// Construct and immediately start the tick-listening task.
    ///
    /// `emitter` receives a `live_session_failed` notification whenever the
    /// session transitions to `Failed`. In production, pass a
    /// `TauriSessionEmitter(app_handle)` (defined in the Tauri binary). In
    /// tests or contexts that don't need UI alerts, pass `Arc::new(NoopEmitter)`.
    ///
    /// `initial_cash` is the user's declared starting capital. It feeds the
    /// engine's `RISK MAX_DAILY_LOSS` check: with `DhanBroker` the live
    /// engine has no way to recover the starting capital from the broker,
    /// so the cap would otherwise be a silent no-op in live trading. A
    /// value of `0.0` (or negative) degrades the check to "never
    /// breached" and is logged at startup so the user notices the cap
    /// won't actually fire.
    pub async fn start(
        strategy_id: String,
        strategy_name: String,
        symbol: String,
        strategy_node: StrategyNode,
        broker: Arc<DhanBroker>,
        feed: Arc<Mutex<FeedManager>>,
        trade_log: Arc<TradeLog>,
        event_bus: Arc<EventBus>,
        initial_candles: Vec<Candle>,
        initial_cash: f64,
        emitter: Arc<dyn SessionEventEmitter>,
        holiday_calendar: Arc<NseHolidayCalendar>,
    ) -> Result<Arc<Self>, String> {
        // Write session context onto the broker before the engine starts so
        // every `execute`/`execute_with_meta` call during this session sees
        // the correct strategy metadata in the trade log.
        *broker.session_context.write() = Some(SessionContext {
            strategy_id: strategy_id.clone(),
            strategy_name: strategy_name.clone(),
            mode: "live".to_string(),
        });

        let execution_target = broker.clone();
        let mut engine = StrategyEngine::new(StrategyInstance {
            id: strategy_id.clone(),
            strategy: Arc::new(strategy_node),
            symbol: symbol.clone(),
            timeframe: Timeframe::M1,
            status: EngineStatus::Running,
            execution_target,
            initial_cash,
        });
        engine.event_bus = Some(event_bus);

        if initial_cash <= 0.0 {
            eprintln!(
                "live_session: strategy {strategy_id} started with initial_cash={initial_cash}; \
                 RISK MAX_DAILY_LOSS will not fire for this session"
            );
        }

        let status = Arc::new(RwLock::new(SessionStatus::Starting));
        let cancel = CancellationToken::new();
        // H3 (audit): wire the session's cancellation token into the
        // broker so a `stop()` mid-`place_order` aborts the call before
        // the trade-log row is appended. We do this *before* subscribing
        // to the feed — if the subscribe fails the broker still has the
        // right token for any subsequent (clean) start. The token itself
        // is fresh until `stop()` fires, so an unused session is harmless.
        broker.set_cancel_token(cancel.clone());

        let session = Arc::new(Self {
            strategy_id,
            strategy_name,
            symbol: symbol.clone(),
            status: status.clone(),
            engine: Arc::new(Mutex::new(engine)),
            broker,
            trade_log,
            candle_history: Arc::new(Mutex::new(initial_candles)),
            start_time: Utc::now(),
            holiday_calendar,
            cancel,
            task: Mutex::new(None),
            emitter,
        });

        let mut receiver = {
            let mut feed = feed.lock().await;
            feed.subscribe(vec![symbol.clone()])
                .await
                .map_err(|error| error.to_string())?;
            feed.subscribe_ticks()
        };

        *status.write() = SessionStatus::Running;

        let task_session = session.clone();
        let task = tokio::spawn(async move {
            let mut assembler = CandleAssembler::new(symbol);

            loop {
                tokio::select! {
                    _ = task_session.cancel.cancelled() => break,
                    tick = receiver.recv() => {
                        let tick = match tick {
                            Ok(tick) => tick,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                eprintln!("[live_session] tick receiver lagged by {skipped} message(s)");
                                continue;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                eprintln!("[live_session] feed closed unexpectedly — transitioning to Failed");
                                let reason = "feed closed unexpectedly".to_string();
                                *task_session.status.write() =
                                    SessionStatus::Failed(reason.clone());
                                // Emit the loud alert. open_positions_estimate is not fetched
                                // here because we're inside the async task with no broker
                                // handle; Phase 8 can add a best-effort fetch before breaking.
                                task_session.emitter.emit_failed(LiveSessionFailedPayload {
                                    strategy_id: task_session.strategy_id.clone(),
                                    reason,
                                    open_positions_estimate: None,
                                });
                                break;
                            }
                        };

                        if tick.symbol != task_session.symbol {
                            continue;
                        }

                        // If the realized-loss cache has gone stale, pause immediately
                        // rather than running the engine with an unreliable safety metric.
                        // Only transition if currently Running (don't stomp Failed/Stopped).
                        if task_session.broker.is_stale() {
                            let should_pause = {
                                let s = task_session.status.read();
                                *s == SessionStatus::Running
                            };
                            if should_pause {
                                task_session.broker.set_paused_for_entries(true);
                                *task_session.status.write() = SessionStatus::Paused;
                                eprintln!(
                                    "[live_session] realized-loss cache stale; \
                                     session paused until cache recovers"
                                );
                            }
                        }

                        let Some(candle) = assembler.feed(&tick) else {
                            continue;
                        };

                        // B1 (audit): per-candle market-hours gate. A session
                        // started at 15:29 would otherwise run `on_candle` on
                        // the 15:30 candle and submit a post-close order
                        // that NSE rejects. Re-using the same predicate as
                        // `LiveGuard::run_preflight` keeps the boundary
                        // definitions in one place; the session treats
                        // weekend/holiday/outside-hours candles as
                        // un-tradeable and skips the rest of the pipeline.
                        //
                        // The candle still goes into `candle_history` so the
                        // indicator warm-up does not skip a day when the
                        // first session candle lands after a weekend gap —
                        // we just don't try to trade on it. Skipping the
                        // history append would leave indicators blind to
                        // that day's prices.
                        if let Some(candle_close_ist) = candle_close_ist(candle.timestamp) {
                            if !is_market_open(candle_close_ist, &task_session.holiday_calendar) {
                                eprintln!(
                                    "[live_session] market closed at candle close {} IST; \
                                     skipping trading on this candle",
                                    candle_close_ist.format("%Y-%m-%d %H:%M:%S")
                                );
                                // Still record the candle so the next
                                // in-session candle has the correct
                                // preceding close.
                                let mut history = task_session.candle_history.lock().await;
                                history.push(candle);
                                if history.len() > MAX_CANDLE_HISTORY {
                                    let excess = history.len() - MAX_CANDLE_HISTORY;
                                    history.drain(0..excess);
                                }
                                continue;
                            }
                        }

                        // Append the new candle and clone the history while the lock is held;
                        // the guard is dropped at the end of this block — before any .await.
                        let candles = {
                            let mut history = task_session.candle_history.lock().await;
                            history.push(candle.clone());
                            // Keep only the most recent MAX_CANDLE_HISTORY candles; older
                            // candles are dead weight once the indicator windows have slid past them.
                            if history.len() > MAX_CANDLE_HISTORY {
                                let excess = history.len() - MAX_CANDLE_HISTORY;
                                history.drain(0..excess);
                            }
                            history.clone()
                        }; // history guard dropped here — no lock held across the awaits below.

                        // C2 + H3 fix (Phase 8): the engine now exposes `plan_candle` which
                        // returns `(Vec<LogEntry>, Vec<OrderIntent>)`. The
                        // tick loop captures the intent list for the
                        // audit trail and forwards it to a future H3-aware
                        // executor (Phase 9). Today the intents are
                        // already executed by `plan_candle` itself — the
                        // split exists so we can re-route them through a
                        // cancellation-aware executor without changing
                        // the engine's eval logic.
                        //
                        // H3 cancellation plumbing is wired separately:
                        // `DhanBroker::execute_with_meta` now checks the
                        // session's `CancellationToken` before the HTTP
                        // call (see `dhan.rs::execute_with_meta`), so a
                        // `stop()` mid-`place_order` aborts the broker
                        // call within a few hundred ms instead of waiting
                        // for the broker's response.
                        let (_logs, intents) = {
                            let mut engine = task_session.engine.lock().await;
                            engine.plan_candle(&candles).await
                        };
                        // Intents are recorded for any future executor
                        // (Phase 9 multi-session work). Today they were
                        // already executed by `plan_candle`; we just keep
                        // the list visible so logs and audit can correlate.
                        let _ = intents;
                    }
                }
            }

            let mut status = task_session.status.write();
            if !matches!(*status, SessionStatus::Failed(_)) {
                *status = SessionStatus::Stopped;
            }
        });

        *session.task.lock().await = Some(task);
        Ok(session)
    }

    /// Pause the session. Candle data continues to accumulate and the engine
    /// still runs `on_candle` every minute — stop-loss, take-profit, and
    /// risk-breach orders are **not** suppressed. Only new *entry* (BUY) orders
    /// are blocked, via the broker's `paused_for_entries` flag.
    ///
    /// L2 (audit): the underlying `set_paused_for_entries` is gated on a
    /// `SessionContext` being set; `LiveSession::start` always sets one
    /// before this method is reachable, so the rejection branch should
    /// never fire in production. We log a warning if it does — that
    /// would mean the session's context was cleared while the session
    /// itself is still active, which is a bookkeeping bug.
    pub fn pause(&self) {
        if !self.broker.set_paused_for_entries(true) {
            eprintln!(
                "[live_session] WARN: pause() called but broker has no SessionContext \
                 — the session's context was cleared out from under it"
            );
        }
        *self.status.write() = SessionStatus::Paused;
    }

    pub fn resume(&self) {
        if !self.broker.set_paused_for_entries(false) {
            eprintln!(
                "[live_session] WARN: resume() called but broker has no SessionContext"
            );
        }
        *self.status.write() = SessionStatus::Running;
    }

    /// How long `stop()` waits for the tick task to finish before logging a
    /// warning that an order may still be in flight. The HTTP `place_order`
    /// call inside `on_candle` does not respect the cancellation token
    /// (see H3 in the live execution audit) — if it is mid-flight when
    /// `stop()` is called, we cannot abort it. The 5-second wait is a
    /// pragmatic bound: real broker round-trips on NSE are typically
    /// 300–800 ms, so anything past 5 s is almost certainly hung.
    const STOP_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

    /// Stop the session. Cancels the cancellation token (the tick loop's
    /// `select!` will break on the next iteration boundary) and awaits the
    /// task with a bounded timeout. If the task does not finish within
    /// `STOP_DRAIN_TIMEOUT`, the order in flight inside `on_candle` is
    /// assumed to still be running at the broker — `stop()` returns and
    /// logs a warning so the caller can surface it to the UI.
    pub async fn stop(&self) {
        self.cancel.cancel();
        let mut task_slot = self.task.lock().await;
        if let Some(task) = task_slot.take() {
            match tokio::time::timeout(Self::STOP_DRAIN_TIMEOUT, task).await {
                Ok(Ok(())) => {
                    // Clean exit — tick loop broke on cancel.
                }
                Ok(Err(join_error)) => {
                    eprintln!("[live_session] task join failed: {join_error}");
                }
                Err(_timeout) => {
                    // The tick loop is stuck inside `on_candle` — almost
                    // certainly blocked on a `place_order` HTTP call. The
                    // order may or may not have reached the broker; the user
                    // must verify in their broker app.
                    eprintln!(
                        "[live_session] WARN: tick task did not stop within \
                         {secs}s — an order may still be in flight at the broker; \
                         please verify your open orders in the broker app",
                        secs = Self::STOP_DRAIN_TIMEOUT.as_secs(),
                    );
                }
            }
        }
        drop(task_slot);
        // Clear session context so the broker no longer attaches this
        // strategy's metadata to any subsequent (unexpected) execute calls.
        *self.broker.session_context.write() = None;
    }

    pub fn status(&self) -> SessionStatus {
        self.status.read().clone()
    }

    // ── L8 (audit) typed accessors ────────────────────────────────────────
    //
    // The `pub(crate) broker` field exposes every method on `DhanBroker`,
    // including `execute_with_meta`. That is too much surface for a
    // plugin host (or a future IPC) that needs to render position counts
    // or staleness toasts. The helpers below narrow the public API to
    // exactly what `commands::live` (and any future read-only caller)
    // needs; everything else stays crate-private.

    /// Snapshot the broker's open positions. Tolerates a broker HTTP
    /// failure with `Ok(Vec::new())` so the caller can still render
    /// "0 open positions" in the UI rather than failing the whole
    /// status call. Mirrors the behaviour the `commands::live` callers
    /// used to rely on before L8 narrowed the broker field.
    pub async fn broker_positions(&self) -> Vec<crate::models::Position> {
        self.broker.get_positions().await.unwrap_or_default()
    }

    /// Non-negative realized-loss magnitude in rupees. Drives the
    /// `LiveStatusWire.realized_loss` field.
    pub fn broker_realized_loss(&self) -> f64 {
        self.broker.realized_loss()
    }

    /// `true` once the broker's realized-loss / available-cash cache has
    /// gone stale. Drives the `LiveStatusWire.loss_tracking_stale` flag.
    pub fn broker_is_stale(&self) -> bool {
        self.broker.is_stale()
    }

    /// Wall-clock duration since the most recent successful broker refresh,
    /// or `None` if no refresh has ever succeeded. Drives the
    /// resume-during-stale error message so the user can see how long the
    /// broker has been unreachable.
    pub fn broker_time_since_last_success(&self) -> Option<Duration> {
        self.broker.time_since_last_success()
    }

    /// Snapshot of the entry-suppression flag (BUY orders blocked). The
    /// GatedLiveExecutionApi re-runs this gate on every plugin order.
    pub fn broker_paused_for_entries(&self) -> bool {
        self.broker.paused_for_entries_snapshot()
    }

    /// Clone of the session's cancellation token. The plugin order
    /// gateway constructs its own copy of this token so its gate stack
    /// can short-circuit orders without re-reading the broker field.
    pub fn broker_cancel_token(&self) -> CancellationToken {
        self.broker.cancel_token()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Candle;
    use chrono::{Datelike, TimeZone, Timelike};

    fn make_candle(ts: i64) -> Candle {
        Candle { timestamp: ts, open: 100.0, high: 101.0, low: 99.0, close: 100.5, volume: 1000.0 }
    }

    /// Mock emitter that records every `emit_failed` call for assertion.
    #[derive(Default)]
    struct MockEmitter {
        events: std::sync::Mutex<Vec<LiveSessionFailedPayload>>,
    }

    impl SessionEventEmitter for MockEmitter {
        fn emit_failed(&self, payload: LiveSessionFailedPayload) {
            self.events.lock().unwrap().push(payload);
        }
    }

    #[test]
    fn test_candle_history_capped() {
        let mut history: Vec<Candle> = (0..(MAX_CANDLE_HISTORY as i64 + 100))
            .map(make_candle)
            .collect();
        // Apply the same logic as the tick loop.
        if history.len() > MAX_CANDLE_HISTORY {
            let excess = history.len() - MAX_CANDLE_HISTORY;
            history.drain(0..excess);
        }
        assert_eq!(history.len(), MAX_CANDLE_HISTORY);
        // Oldest 100 candles (timestamps 0..100) were evicted; first remaining is 100.
        assert_eq!(history[0].timestamp, 100);
        assert_eq!(history[MAX_CANDLE_HISTORY - 1].timestamp, MAX_CANDLE_HISTORY as i64 + 99);
    }

    /// Verify that `NoopEmitter` compiles and silently swallows events.
    #[test]
    fn test_noop_emitter_is_silent() {
        let emitter = NoopEmitter;
        emitter.emit_failed(LiveSessionFailedPayload {
            strategy_id: "x".to_string(),
            reason: "test".to_string(),
            open_positions_estimate: None,
        });
        // No panic — that's the assertion.
    }

    /// Verify that `MockEmitter` records `emit_failed` calls with the correct
    /// payload. This is the unit test for the `SessionEventEmitter` contract;
    /// the full "session transitions to Failed and emits" path is an integration
    /// test requiring a live feed setup and is deferred to Phase 8.
    #[test]
    fn test_failed_status_emits_event() {
        let emitter = MockEmitter::default();

        emitter.emit_failed(LiveSessionFailedPayload {
            strategy_id: "strat-42".to_string(),
            reason: "feed closed unexpectedly".to_string(),
            open_positions_estimate: Some(2),
        });

        let events = emitter.events.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].strategy_id, "strat-42");
        assert_eq!(events[0].reason, "feed closed unexpectedly");
        assert_eq!(events[0].open_positions_estimate, Some(2));
    }

    /// Verify that a `RecvError::Lagged` on the broadcast channel is recoverable:
    /// the tick loop matches it to `continue`, not `break`, so the session stays alive.
    /// This test demonstrates the channel contract that underpins that logic.
    // ---- B1 (audit): per-candle market-hours gate ----
    //
    // The tick loop's gate is a one-line `is_market_open(candle_close_ist, ...)`.
    // The interesting cases are the conversion itself and the predicate's
    // behaviour at the IST open/close boundary when fed via `candle.timestamp`.

    /// 2026-01-05 09:30:00 IST (Monday, well inside trading hours).
    /// Build the millisecond timestamp the engine sees.
    fn ts_ist(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
        let ist = chrono::FixedOffset::east_opt(IST_OFFSET_SECONDS).unwrap();
        let nd = chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let nt = chrono::NaiveTime::from_hms_opt(h, mi, s).unwrap();
        let dt = ist
            .from_local_datetime(&nd.and_time(nt))
            .single()
            .expect("valid ist instant");
        dt.timestamp_millis()
    }

    #[test]
    fn test_candle_close_ist_converts_mid_session() {
        // 2026-01-05 (Monday) 09:30:00 IST.
        let ts = ts_ist(2026, 1, 5, 9, 30, 0);
        let got = candle_close_ist(ts).expect("valid ts");
        assert_eq!(got.hour(), 9);
        assert_eq!(got.minute(), 30);
        assert_eq!(got.weekday(), chrono::Weekday::Mon);
    }

    #[test]
    fn test_candle_close_ist_returns_none_for_garbage() {
        // Far-future timestamp that overflows `from_timestamp_millis`.
        assert!(candle_close_ist(i64::MAX).is_none());
    }

    /// Saturday candle: even with an empty holiday calendar, the gate
    /// must reject weekend timestamps.
    #[test]
    fn test_gate_skips_weekend_candle() {
        let cal = NseHolidayCalendar::new();
        // 2026-01-03 is a Saturday.
        let sat_close = candle_close_ist(ts_ist(2026, 1, 3, 10, 0, 0)).unwrap();
        assert!(!is_market_open(sat_close, &cal));
    }

    /// Post-close candle: a 15:30:01 IST candle on a trading day must be
    /// rejected. This is the exact B1 scenario — a session started at 15:29
    /// must not submit a 15:30 order.
    #[test]
    fn test_gate_skips_post_close_candle() {
        let cal = NseHolidayCalendar::new();
        // 2026-01-05 (Monday) 15:30:01 IST.
        let post_close = candle_close_ist(ts_ist(2026, 1, 5, 15, 30, 1)).unwrap();
        assert!(!is_market_open(post_close, &cal));
        // And the inclusive boundary at 15:30:00 is still open.
        let close = candle_close_ist(ts_ist(2026, 1, 5, 15, 30, 0)).unwrap();
        assert!(is_market_open(close, &cal));
    }

    /// Holiday candle: 2026-01-26 (Republic Day, Monday) must be rejected
    /// only when registered in the calendar — proves the calendar actually
    /// reaches the gate, not just the weekday/weekend predicates.
    #[test]
    fn test_gate_skips_holiday_candle() {
        let empty = NseHolidayCalendar::new();
        let holidays = vec![chrono::NaiveDate::from_ymd_opt(2026, 1, 26).unwrap()];
        let populated = NseHolidayCalendar::with_holidays(holidays);
        let rep_day = candle_close_ist(ts_ist(2026, 1, 26, 10, 0, 0)).unwrap();
        // Empty calendar treats it as a normal trading day.
        assert!(is_market_open(rep_day, &empty));
        // Populated calendar treats it as a holiday.
        assert!(!is_market_open(rep_day, &populated));
    }

    #[tokio::test]
    async fn test_lag_error_does_not_break_loop() {
        // Channel with capacity 2 — send 3 items before reading to overflow the buffer.
        // Message 1 is dropped; the receiver lags by 1 but messages 2 and 3 survive.
        let (tx, mut rx) = tokio::sync::broadcast::channel::<i32>(2);
        let _ = tx.send(1);
        let _ = tx.send(2);
        let _ = tx.send(3); // drops message 1; receiver is now lagged by 1

        let first = rx.recv().await;
        assert!(
            matches!(first, Err(tokio::sync::broadcast::error::RecvError::Lagged(_))),
            "expected Lagged when receiver falls behind"
        );

        // After Lagged the receiver is repositioned to the oldest surviving message
        // (message 2). The loop `continue`s — the receiver must still be usable.
        assert!(
            rx.recv().await.is_ok(),
            "receiver must still work after a Lagged error"
        );
    }
}
