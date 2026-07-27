use serde::Serialize;
use std::sync::Arc;
use tauri::State;

use crate::{
    broker::Timeframe,
    commands::{
        state::AppState,
        strategy::parse_and_validate_dsl,
    },
    live::{
        guard::{LiveGuard, LiveGuardResult},
        session::{
            LiveSession,
            SessionEventEmitter,
        },
        trade_log::{TradeLog, TradeLogEntry},
    },
    strategy::dsl::TradeIn,
};

/// Returns all entries from the trade log, newest first.
pub async fn get_trade_log(state: State<'_, AppState>) -> Result<Vec<TradeLogEntry>, String> {
    TradeLog::read_all(&state.trade_log_path)
        .map(|mut v| {
            v.reverse();
            v
        })
        .map_err(|e| e.to_string())
}

// =============================================================================
// Phase 7 — Live start / confirm / acknowledge
// =============================================================================

/// Wire shape returned by `request_live_start`. The UI keeps the token and
/// passes it back to `confirm_live_start` within the TTL. `requires_ack` is
/// set when gates 1–8 pass but the user has not yet acknowledged
/// live-trading risks.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLiveStartResult {
    pub token: String,
    pub requires_ack: bool,
    /// The symbol that the strategy will trade. Echoed back so the UI can
    /// display it in the confirmation dialog alongside the strategy name.
    pub symbol: String,
}

/// Begin a live session. Validates gates 1–8 via [`LiveGuard::run_preflight`]
/// and stores a fresh 90-second token in `AppState.pending_live_token`. The
/// token — not the session — is what is returned to the UI.
///
/// `acknowledged_live_trading` is a separate end-point that just writes
/// the consent file; tokens never need to know about it.
pub async fn request_live_start(
    state: State<'_, AppState>,
    strategy_id: String,
) -> Result<RequestLiveStartResult, String> {
    // Reject if a session is already active. We hold the slot lock across
    // the whole call so two simultaneous requests cannot both pass.
    let session = state.live_session.lock().await;
    if session.is_some() {
        return Err(
            "a live session is already active; stop it before starting another".to_string(),
        );
    }
    drop(session);

    // Look up the deployed strategy record. The wire `DeployedStrategy`
    // collapses `mode` into `modes: Vec<StrategyMode>`; we want the
    // authoritative record to check the on-disk mode exactly.
    let record = state
        .strategies
        .get(&strategy_id)
        .await?
        .ok_or_else(|| format!("strategy not found: {strategy_id}"))?;

    // Gate 1 (paper-default guard): live requires the record to have been
    // deployed with `mode = Live`. Paper is the default.
    if record.mode != crate::commands::registry::StrategyMode::Live {
        return Err(
            "strategy was not deployed in Live mode; redeploy with mode 'live' to enable \
             live trading"
                .to_string(),
        );
    }

    // Parse + validate the DSL so we have the AST for the gates that read it
    // (6 + 7: risk controls). The same parse is also re-run inside
    // `confirm_live_start` for the session builder — there is no cache;
    // the cost is negligible.
    let strategy_node = parse_and_validate_dsl(&record.dsl_source)?;

    // Resolve the symbol from the DSL's `TRADE_IN` clause. Phase 7 only
    // supports a single explicit symbol per strategy; index-based multi-
    // symbol live sessions are out of scope.
    let symbol = match &strategy_node.trade_in {
        Some(TradeIn::Symbols(symbols)) if symbols.len() == 1 => {
            symbols.first().cloned().unwrap()
        }
        Some(TradeIn::Symbols(_)) => {
            return Err(
                "live mode in Phase 7 supports exactly one TRADE_IN symbol; \
                 multi-symbol strategies are not yet supported for live trading"
                    .to_string(),
            )
        }
        Some(TradeIn::Index(_)) => {
            return Err(
                "live mode in Phase 7 does not yet support TRADE_IN NIFTY_* \
                 indexes; use an explicit single TRADE_IN SYMBOLS clause"
                    .to_string(),
            )
        }
        None => {
            return Err(
                "strategy has no TRADE_IN clause; add `TRADE_IN RELIANCE` \
                 (or another single symbol) before starting a live session"
                    .to_string(),
            )
        }
    };

    // Run gates 1–8 via the guard. The guard issues the token itself.
    let result = state
        .live_guard
        .run_preflight(&symbol, &strategy_id, &strategy_node)
        .await?;

    let (token, requires_ack) = match result {
        LiveGuardResult::Ok { token } => (token, false),
        LiveGuardResult::RequiresAcknowledgment { token } => (token, true),
    };
    let token_string = token.token.clone();

    // Store the token in the slot. A new request overwrites any prior
    // pending token — the newer request wins; older token is invalidated.
    *state.pending_live_token.lock().await = Some(token);

    Ok(RequestLiveStartResult {
        token: token_string,
        requires_ack,
        symbol,
    })
}

/// Validate the pending token, then build and start the live session. Held
/// lock discipline: `state.live_session` is taken before the token check so
/// a concurrent `request_live_start` cannot slip a second session in
/// between the validation and the insert.
///
/// The token is consumed (cleared) immediately after validation regardless
/// of whether the session start succeeds — the token is single-use.
pub async fn confirm_live_start(
    state: State<'_, AppState>,
    strategy_id: String,
    token: String,
) -> Result<(), String> {
    // 1. Take the session lock first and hold it across the entire call
    //    so a concurrent `confirm_live_start` (or `request_live_start`)
    //    cannot race us.
    let mut session_slot = state.live_session.lock().await;
    if session_slot.is_some() {
        return Err("a live session is already active".to_string());
    }

    // 2. Validate + clear the token. The token slot is `tokio::sync::Mutex`
    //    so we don't need to hold the parking_lot guard across `.await`s.
    {
        let mut pending = state.pending_live_token.lock().await;
        LiveGuard::validate_token(&*pending, &strategy_id, &token)?;
        *pending = None;
    }

    // 3. Re-fetch the strategy and re-parse the DSL (it could have changed
    //    since `request_live_start` was called, e.g. via `deploy_strategy`
    //    with the same id — though id is unique per deploy). The cost is
    //    trivial; the determinism benefit (no stale AST in the session) is
    //    worth it.
    let record = state
        .strategies
        .get(&strategy_id)
        .await?
        .ok_or_else(|| format!("strategy not found: {strategy_id}"))?;
    if record.mode != crate::commands::registry::StrategyMode::Live {
        return Err(
            "strategy was not deployed in Live mode; redeploy with mode 'live' to enable \
             live trading"
                .to_string(),
        );
    }

    let strategy_node = parse_and_validate_dsl(&record.dsl_source)?;
    let symbol = match &strategy_node.trade_in {
        Some(TradeIn::Symbols(symbols)) if symbols.len() == 1 => {
            symbols.first().cloned().unwrap()
        }
        _ => {
            return Err(
                "live mode in Phase 7 supports exactly one TRADE_IN symbol".to_string(),
            )
        }
    };

    // 4. Fetch seed candles. Tolerate failure with a stderr warning; the
    //    engine simply starts cold with whatever it has. The full seed is
    //    a "nice to have" for indicator warm-up, not a safety precondition.
    let seed = match fetch_seed_candles(&state, &symbol).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "confirm_live_start: seed OHLCV fetch failed for {symbol}: {e}; starting cold"
            );
            Vec::new()
        }
    };

    // 5. Start the session. Wire the Tauri app handle as the failure-event
    //    emitter so loud alerts surface in the UI. The session writes a
    //    SessionContext onto the broker before the first execute, so the
    //    trade log entry carries the strategy metadata.
    let emitter: Arc<dyn SessionEventEmitter> = Arc::new(TauriSessionEmitter::new(
        state.app_handle.clone(),
    ));
    let session = LiveSession::start(
        strategy_id.clone(),
        record.name.clone(),
        symbol.clone(),
        strategy_node,
        state.live_guard.broker.clone(),
        state.data.feed.clone(),
        state.trade_log.clone(),
        state.event_bus.clone(),
        seed,
        emitter,
    )
    .await?;

    // 6. Insert into the slot. Lock is held — no other caller can race.
    *session_slot = Some(session);
    Ok(())
}

/// Write (or rewrite) the persistent "live trading acknowledged" file. A
/// one-time consent that survives across sessions and reboots. After this
/// runs, `request_live_start` will return `requires_ack: false`.
pub async fn acknowledge_live_trading(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let payload = serde_json::json!({
        "acknowledged": true,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    let serialized = serde_json::to_string_pretty(&payload)
        .map_err(|e| format!("failed to serialize ack: {e}"))?;
    std::fs::write(&state.ack_path, serialized)
        .map_err(|e| format!("failed to write ack file: {e}"))
}

/// Adapter from `tauri::AppHandle` to the library's `SessionEventEmitter`
/// trait. Lives here (rather than `live::session`) because the `tauri`
/// dependency only resolves in the binary crate; the library only sees
/// the trait.
struct TauriSessionEmitter {
    handle: tauri::AppHandle,
}

impl TauriSessionEmitter {
    fn new(handle: tauri::AppHandle) -> Self {
        Self { handle }
    }
}

impl SessionEventEmitter for TauriSessionEmitter {
    fn emit_failed(&self, payload: crate::live::session::LiveSessionFailedPayload) {
        use tauri::Emitter;
        let _ = self.handle.emit("live-session-failed", &payload);
    }
}

async fn fetch_seed_candles(
    state: &AppState,
    symbol: &str,
) -> Result<Vec<crate::models::Candle>, String> {
    let to = chrono::Utc::now().timestamp_millis();
    // 500 bars of 1-minute data is roughly a day — a reasonable warm-up
    // window. Falls back gracefully (logged warning, empty vec) on error.
    let from = to - 500 * 60 * 1_000;
    state
        .data
        .broker
        .get_ohlcv(symbol, Timeframe::M1, from, to)
        .await
        .map_err(|e| e.to_string())
}
