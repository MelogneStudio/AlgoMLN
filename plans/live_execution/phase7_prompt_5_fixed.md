# AlgoMLN — Phase 7, Prompt 5 (Fixed): Tauri IPC + AppState + Event Bus

Run this prompt in Claude Code / Cursor / Windsurf against the existing repo.
This is the fixed version of Phase 7 Prompt 5.

Output code only — no explanations, no questions, unless explicitly told to ask.

---

## Environment note (read first)

This machine does **not** have the Rust toolchain, Node, or npm installed.
You cannot run `cargo test`, `cargo build`, `cargo check`, `npm run build`,
or `npm run dev` yourself. Whenever you would normally run one:

1. Write the code change first.
2. State the exact command you want executed.
3. Ask the user to run it and paste back the full output.
4. Wait for the results before moving on.
5. If a command fails, ask for the complete error text — do not guess.

Do not attempt to install toolchains. Do not skip verification steps.

---

## What already exists (context for a fresh AI)

Prompts 1–4 of Phase 7, plus a fix pack for Prompts 1–3, have been
implemented. You can assume the following:

- `src/broker/dhan/rest.rs` — `DhanClient` with `place_order`
  (correlationId, no auto-retry), `get_positions`, OHLCV methods.
- `src/strategy/execution/dhan.rs` — `DhanBroker` implementing
  `ExecutionTarget`, with `execute_with_meta(order, rule_id, notes)`,
  `realized_loss()` (non-negative magnitude), `is_stale()`, background
  positions refresh, owns `Arc<TradeLog>` and a `SessionContext` slot.
- `src/live/trade_log.rs` — `TradeLog`, `TradeLogEntry`.
- `src/live/candle_assembler.rs` — 1-minute assembler.
- `src/live/session.rs` — `LiveSession` with pause-blocks-entries
  semantics, bounded `candle_history`, `broadcast::Lagged` handling,
  Tauri event emit on `Failed`.
- `src/live/guard.rs` — `LiveGuard`, `PendingLiveToken`,
  `is_market_open`, `LiveGuardResult` (`Ok { token }` /
  `RequiresAcknowledgment { token }`).
- `src/live/holidays.rs` — `NseHolidayCalendar`.
- `src/commands/live.rs` — `get_trade_log`, `request_live_start`,
  `confirm_live_start`, `acknowledge_live_trading`. These need to be
  extended with pause/resume/stop/status in this prompt.
- `src/commands/state.rs` — `AppState` with all fields declared but not
  yet wired in `main.rs`.

If any of the above does not exist as described, stop and ask.

---

## Task

Finalise all Tauri wiring. Also: because plugin-issued orders would
bypass every safety gate (`MAX_DAILY_LOSS` accounting, market hours,
etc.), the plugin `Execution` capability is **explicitly disabled** in
Phase 7. See Part D below.

### A. `src/commands/state.rs` — finalise `AppState`

Ensure the final struct is:

```rust
pub struct AppState {
    // Existing:
    pub data: DataState,
    pub registry: Arc<StrategyRegistry>,
    pub plugin_registry: Arc<PluginRegistry>,
    pub ui_rx: tokio::sync::broadcast::Receiver<UiMessage>,
    pub event_bus: Arc<EventBus>,

    // Phase 7:
    pub trade_log: Arc<TradeLog>,
    pub trade_log_path: PathBuf,
    pub live_session: Arc<parking_lot::Mutex<Option<Arc<LiveSession>>>>,
    pub live_guard: Arc<LiveGuard>,
    pub pending_live_token: Arc<parking_lot::Mutex<Option<PendingLiveToken>>>,
    pub ack_path: PathBuf,
    pub app_handle: tauri::AppHandle,
}
```

Deduplicate any fields already added in earlier prompts. `app_handle`
is required for event emission on failure/stop.

### B. `src-tauri/src/main.rs` — `setup` closure

Add after existing setup code:

```rust
let app_data_dir = app.path_resolver().app_data_dir()
    .expect("no app data dir");
std::fs::create_dir_all(&app_data_dir).ok();

// Trade log
let trade_log_path = app_data_dir.join("trade_log.jsonl");
let trade_log = Arc::new(
    TradeLog::open(trade_log_path.clone())
        .expect("failed to open trade log")
);

// Live guard
let ack_path = app_data_dir.join("live_ack.json");
let holiday_calendar = Arc::new(NseHolidayCalendar::new());
let live_guard = Arc::new(LiveGuard {
    client: dhan_client.clone(),
    symbol_map: symbol_map.clone(),
    broker: dhan_broker.clone(),   // constructed earlier in setup
    ack_path: ack_path.clone(),
    holiday_calendar,
});

// Pending token slot + session slot
let pending_live_token = Arc::new(parking_lot::Mutex::new(None));
let live_session = Arc::new(parking_lot::Mutex::new(None));

// AppHandle for event emission
let app_handle = app.handle();
```

Wire `dhan_broker` into `DataState` if not already there (the Prompt 1
fix pack assumed this). `DhanBroker` must be constructible before
`live_session` because `LiveGuard` needs it.

Include all Phase 7 fields when constructing `AppState`.

### C. Event bus — live engine wiring

`LiveSession::start` already receives `event_bus: Arc<EventBus>` and
assigns it via `engine.set_event_bus(Some(event_bus))`. Verify this is
present. If it was left as a TODO, implement it now — the engine already
has `event_bus: Option<Arc<EventBus>>` and publishes `RuleFired`,
`TradeExecuted`, `CandleProcessed` when `Some`.

`run_backtest_internal` continues to pass `None`. Do not relax this.
Backtests must never publish to the plugin event bus.

### D. Plugin `Execution` capability — disabled in Phase 7

**Do not** implement `DhanBrokerExecutionApi` with a live `submit_order`.
Plugin-issued orders would bypass:

- `MAX_DAILY_LOSS` gating (the engine's per-candle risk check)
- Session pause (paused plugin still submits entries)
- Market-hours gate (`LiveGuard` only runs at session start)
- Trade-log strategy context (plugin has no `SessionContext`)

Instead, implement a **read-only** live execution API. Plugins can query
positions but cannot submit orders in Phase 7. Phase 8 introduces a
proper risk-checking plugin order gateway.

```rust
// src/plugin/api/execution.rs — alongside NoopExecutionApi:

/// Read-only live execution API. Plugins can inspect positions during
/// a live session but cannot submit orders in Phase 7. See CLAUDE.md
/// invariant 18.
pub struct ReadOnlyLiveExecutionApi {
    broker: Arc<DhanBroker>,
    handle: tokio::runtime::Handle,
}

impl ExecutionApi for ReadOnlyLiveExecutionApi {
    fn submit_order(&self, _symbol: &str, _side: &str, _qty: i64)
        -> Result<String, PluginError>
    {
        Err(PluginError::ApiError(
            "plugin order submission is disabled in Phase 7; \
             live orders may only originate from the strategy engine".into()
        ))
    }

    fn cancel_order(&self, _order_id: &str) -> Result<(), PluginError> {
        Err(PluginError::ApiError("cancel not supported in Phase 7".into()))
    }

    fn positions(&self) -> Result<Vec<PluginPosition>, PluginError> {
        // Async → sync bridge. block_on inside a running tokio runtime
        // panics; use block_in_place + Handle::block_on, or use
        // tokio::runtime::Handle::current().block_on inside a
        // spawn_blocking. Preferred here:
        tokio::task::block_in_place(|| {
            self.handle.block_on(async {
                self.broker.get_positions()
                    .await
                    .map_err(|e| PluginError::ApiError(e.to_string()))
                    .map(|ps| ps.into_iter().map(Into::into).collect())
            })
        })
    }
}
```

`tokio::task::block_in_place` requires the multi-thread runtime; the
Tauri app uses multi-thread by default. If for any reason the plugin
host runs on a current-thread runtime, this will panic — add a debug
assertion and document.

**Runtime handle:** capture `tokio::runtime::Handle::current()` in the
`setup` closure and pass it into the `HostFactory` closure so
`ReadOnlyLiveExecutionApi` gets it at construction time. Never call
`Handle::current()` from inside a plugin callback — the plugin may be
on a non-tokio thread.

Update the `HostFactory` closure in `main.rs`:

```rust
let live_session_slot = live_session.clone();
let runtime_handle = tokio::runtime::Handle::current();

let host_factory = move || {
    let exec: Arc<dyn ExecutionApi> = match &*live_session_slot.lock() {
        Some(session) => Arc::new(ReadOnlyLiveExecutionApi {
            broker: session.broker.clone(),
            handle: runtime_handle.clone(),
        }),
        None => Arc::new(NoopExecutionApi),
    };
    // ... build PluginHost with exec ...
};
```

### E. `src/commands/live.rs` — pause/resume/stop/status

Add:

```rust
pub async fn pause_live_strategy(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let slot = state.live_session.lock();
    let session = slot.as_ref()
        .ok_or_else(|| "no live session".to_string())?;
    session.pause();
    Ok(())
}

pub async fn resume_live_strategy(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let slot = state.live_session.lock();
    let session = slot.as_ref()
        .ok_or_else(|| "no live session".to_string())?;
    session.resume();
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopResult {
    pub stopped: bool,
    /// Warning about state the user must handle manually.
    /// e.g. "5 open positions and 2 pending orders remain in NIFTY —
    /// close them manually in your broker app."
    pub open_positions_warning: Option<String>,
}

pub async fn stop_live_strategy(
    state: State<'_, AppState>,
) -> Result<StopResult, String> {
    // 1. Take the session out of the slot.
    let session = state.live_session.lock().take()
        .ok_or_else(|| "no live session".to_string())?;

    // 2. Query positions BEFORE stopping (broker call).
    //    Also query pending orders if the API supports it (Dhan does).
    //    For Phase 7, we report positions + note that pending orders
    //    are not tracked here; the user must check their broker app.
    let positions = session.broker.get_positions().await
        .unwrap_or_default();
    let open_count = positions.iter().filter(|p| p.quantity != 0).count();

    // 3. Stop the session (cancels tick loop, awaits task exit).
    session.stop().await;

    // 4. Emit an event so the UI toasts even if the user isn't looking
    //    at the Live screen.
    let warning = if open_count > 0 {
        let msg = format!(
            "WARNING: {} open position(s) remain — close them manually in your broker app. \
             Pending limit orders are not auto-cancelled; please verify in your broker app.",
            open_count
        );
        state.app_handle.emit_all("live_session_stopped_with_positions",
            serde_json::json!({ "warning": msg.clone() })).ok();
        Some(msg)
    } else {
        None
    };

    Ok(StopResult { stopped: true, open_positions_warning: warning })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStatusWire {
    pub strategy_id: String,
    pub strategy_name: String,
    pub symbol: String,
    /// "Starting" | "Running" | "Paused" | "Stopped" | "Failed"
    pub status: String,
    pub fail_reason: Option<String>,
    pub start_time: String,
    pub position_count: i64,
    /// Non-negative magnitude of session-realized losses in rupees.
    pub realized_loss: f64,
    /// True if the broker's loss-tracking refresh is failing.
    pub loss_tracking_stale: bool,
}

pub async fn get_live_status(
    state: State<'_, AppState>,
) -> Result<Option<LiveStatusWire>, String> {
    let slot = state.live_session.lock();
    let session = match &*slot {
        Some(s) => s.clone(),
        None => return Ok(None),
    };
    drop(slot); // release before awaits

    let positions = session.broker.get_positions().await.unwrap_or_default();
    let (status_str, fail_reason) = match session.status() {
        SessionStatus::Starting => ("Starting".into(), None),
        SessionStatus::Running => ("Running".into(), None),
        SessionStatus::Paused => ("Paused".into(), None),
        SessionStatus::Stopped => ("Stopped".into(), None),
        SessionStatus::Failed(r) => ("Failed".into(), Some(r)),
    };

    Ok(Some(LiveStatusWire {
        strategy_id: session.strategy_id.clone(),
        strategy_name: session.strategy_name.clone(),
        symbol: session.symbol.clone(),
        status: status_str,
        fail_reason,
        start_time: session.start_time.to_rfc3339(),
        position_count: positions.iter().filter(|p| p.quantity != 0).count() as i64,
        realized_loss: session.broker.realized_loss(),
        loss_tracking_stale: session.broker.is_stale(),
    }))
}
```

### F. `src-tauri/src/main.rs` — command wrappers

Add `#[tauri::command]` wrappers for each and register in
`tauri::generate_handler!`:

- `request_live_start`
- `confirm_live_start`
- `acknowledge_live_trading`
- `pause_live_strategy`
- `resume_live_strategy`
- `stop_live_strategy`
- `get_live_status`
- `get_trade_log`

Pattern: one-line wrapper delegating to `commands::live::*`.

### G. `src/types/tauri.ts` — frontend wrappers

Add:

```typescript
export async function requestLiveStart(
    strategyId: string
): Promise<RequestLiveStartResult> {
    if (!isTauri()) {
        throw new Error("live trading is not available in the browser");
    }
    return invoke("request_live_start", { strategyId });
}

export async function confirmLiveStart(
    strategyId: string, token: string
): Promise<void> {
    if (!isTauri()) {
        throw new Error("live trading is not available in the browser");
    }
    return invoke("confirm_live_start", { strategyId, token });
}

export async function acknowledgeLiveTrading(): Promise<void> {
    if (!isTauri()) {
        throw new Error("live trading is not available in the browser");
    }
    return invoke("acknowledge_live_trading");
}

export async function pauseLiveStrategy(): Promise<void> {
    if (!isTauri()) return;
    return invoke("pause_live_strategy");
}

export async function resumeLiveStrategy(): Promise<void> {
    if (!isTauri()) return;
    return invoke("resume_live_strategy");
}

export async function stopLiveStrategy(): Promise<StopResult> {
    if (!isTauri()) return { stopped: false, openPositionsWarning: null };
    return invoke("stop_live_strategy");
}

export async function getLiveStatus(): Promise<LiveStatusWire | null> {
    if (!isTauri()) return null;
    return invoke("get_live_status");
}

export async function getTradeLog(): Promise<TradeLogEntry[]> {
    if (!isTauri()) return [];
    return invoke("get_trade_log");
}
```

### H. `src/types/live.ts` — new file

```typescript
export interface RequestLiveStartResult {
    token: string;
    requiresAck: boolean;
}

export interface StopResult {
    stopped: boolean;
    openPositionsWarning: string | null;
}

export interface LiveStatusWire {
    strategyId: string;
    strategyName: string;
    symbol: string;
    status: 'Starting' | 'Running' | 'Paused' | 'Stopped' | 'Failed';
    failReason: string | null;
    startTime: string;
    positionCount: number;
    realizedLoss: number;
    lossTrackingStale: boolean;
}

export interface TradeLogEntry {
    id: string;
    timestamp: string;
    strategyId: string;
    strategyName: string;
    symbol: string;
    side: 'BUY' | 'SELL';
    quantity: number;
    price: number;
    orderId: string;
    orderStatus: string;
    mode: string;
    ruleId: string;
    notes: string;
}

export interface LiveSessionFailedPayload {
    strategyId: string;
    reason: string;
    openPositionsEstimate: number | null;
}

export interface LiveSessionStoppedPayload {
    warning: string;
}
```

---

## After coding

Ask the user to run:

```
cargo test --lib
cargo build --release
npm run build
```

Wait for output. Do not proceed to Prompt 6 until all pass.

If `npm run build` reports TypeScript errors, ask for the full error
output.

---

## Ask the user (before finalising)

Before finalising Part D, confirm with the user:

> "Phase 7 disables plugin order submission (`submit_order`) — plugins
> can only inspect positions during a live session, not place orders.
> Live orders can only originate from the strategy engine, so
> `MAX_DAILY_LOSS` and market-hours gates cover every real order. A
> proper plugin order gateway with the same gates is Phase 8 work. Is
> this acceptable? (Y/n)"

If the user says no, stop and ask how they want plugin orders gated.
Do not silently implement a live plugin `submit_order`.
