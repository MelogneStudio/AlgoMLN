# AlgoMLN — Phase 7: Live Trading
## Implementation Prompts (Conservative / Safeguards-First)

Run these prompts in order in Claude Code, Cursor, or Windsurf.
Each prompt is self-contained. Do not skip or reorder.
Output code only — no explanations, no questions.

---

## Prompt 1 — DhanBroker: place_order + get_positions + ExecutionTarget impl

### Context

`src/broker/dhan/rest.rs` already has a working `post<T>()` helper used by `get_ohlcv_intraday`, `get_ohlcv`, and `get_quote`. It handles auth headers and error mapping.

`src/strategy/execution/target.rs` defines:

```rust
#[async_trait]
pub trait ExecutionTarget: Send + Sync {
    async fn execute(&self, order: Order) -> Result<OrderResult, ExecutionError>;
    async fn get_positions(&self) -> Result<Vec<Position>, ExecutionError>;
    fn realized_loss(&self) -> f64;
    fn available_cash(&self) -> f64;
    fn is_paper(&self) -> bool;
    fn name(&self) -> &str;
    fn as_any(&self) -> &dyn Any;
}
```

`src/models/` defines `Order`, `OrderResult`, `Position`. The `Order` struct has at minimum: `symbol: String`, `quantity: i64`, `side: OrderSide` (`Buy`/`Sell`), `order_type: OrderType` (`Market`/`Limit`), `price: Option<f64>`.

`src/broker/dhan/models.rs` has Dhan-specific request/response shapes already.

### Task

**A. `src/broker/dhan/rest.rs`** — implement the two stub methods on `DhanClient`:

```
place_order(&self, order: Order) -> Result<OrderResult>
get_positions(&self) -> Result<Vec<Position>>
```

`place_order` maps to `POST /orders`. The Dhan order placement body shape is:

```json
{
  "dhanClientId": "<client_id>",
  "transactionType": "BUY" | "SELL",
  "exchangeSegment": "NSE_EQ",
  "productType": "INTRADAY",
  "orderType": "MARKET",
  "validity": "DAY",
  "securityId": "<dhan_security_id>",
  "quantity": 1,
  "price": 0,
  "triggerPrice": 0,
  "disclosedQuantity": 0,
  "afterMarketOrder": false,
  "amoTime": "OPEN",
  "boProfitValue": 0,
  "boStopLossValue": 0
}
```

- `securityId` must be resolved from `SymbolMap`. Add `symbol_map: Arc<parking_lot::RwLock<SymbolMap>>` to `DhanClient` (it is already in `AppState`; thread it through from `DataState`). If the symbol is not found in the map, return `anyhow::bail!("symbol not in map: {symbol}")`.
- `transactionType` maps from `OrderSide`.
- `orderType` maps from `OrderType`; for `Limit` orders, `price` is the limit price.
- `productType` is always `"INTRADAY"` for now — live trading is intraday only in Phase 7.
- `validity` is always `"DAY"`.
- The response body is `{ "orderId": "<string>", "orderStatus": "TRANSIT" | "PENDING" | "REJECTED" | "CANCELLED" | "TRADED" | ... }`. Map to `OrderResult { order_id: String, status: OrderStatus }`.
- If `orderStatus` is `"REJECTED"` or `"CANCELLED"`, return `Err(anyhow!("order rejected: {status}"))`.

`get_positions` maps to `GET /positions`. The response is an array of position objects:

```json
[{
  "securityId": "...",
  "tradingSymbol": "NIFTY",
  "exchangeSegment": "NSE_EQ",
  "productType": "INTRADAY",
  "buyAvg": 21500.0,
  "buyQty": 10,
  "sellAvg": 0.0,
  "sellQty": 0,
  "netQty": 10,
  "realizedProfit": 0.0,
  "unrealizedProfit": 500.0,
  "dayBuyValue": 215000.0,
  "daySellValue": 0.0
}]
```

Map to `Vec<Position>` using `tradingSymbol` as `symbol`, `netQty` as `quantity`, `buyAvg` as `average_price`, `realizedProfit` as `realized_pnl`, `unrealizedProfit` as `unrealized_pnl`. Skip entries where `netQty == 0`.

**B. `src/strategy/execution/dhan.rs`** — new file. Implement `DhanBroker`:

```rust
pub struct DhanBroker {
    client: Arc<DhanClient>,
    realized_loss: Mutex<f64>,
}

impl DhanBroker {
    pub fn new(client: Arc<DhanClient>) -> Self { ... }
}
```

Implement `ExecutionTarget` for `DhanBroker`:

- `execute`: calls `client.place_order(order)`, maps `anyhow::Error` to `ExecutionError::BrokerError(msg)`. On success, accumulates negative realized PnL via `get_positions` differential (this is approximate for now — Phase 7 will track this more precisely in the trade log). For Phase 7, `realized_loss` just increments by `price * quantity` for sells that close positions — do a simple position snapshot before and after and diff `realized_pnl`. If that fails, treat as 0 additional loss (never panic, log to stderr).
- `get_positions`: calls `client.get_positions()`, maps errors to `ExecutionError::BrokerError`.
- `realized_loss`: returns `*self.realized_loss.lock()`.
- `available_cash`: return `f64::MAX` — Dhan does not expose free cash via this API; risk is enforced by the engine's `RISK MAX_DAILY_LOSS` on realized loss, not by a cash balance.
- `is_paper`: `false`.
- `name`: `"dhan"`.
- `as_any`: standard `self`.

**C. `src/strategy/execution/mod.rs`** — add `pub mod dhan;` and re-export `DhanBroker`.

**D. `src/broker/dhan/models.rs`** — add the Dhan-specific request/response structs for order placement and positions if they do not already exist (`PlaceOrderRequest`, `PlaceOrderResponse`, `DhanPosition`).

**E. Unit tests** — in `src/strategy/execution/dhan.rs`, add `#[cfg(test)] mod tests` with at least:
- `test_dhan_broker_is_not_paper`: asserts `is_paper() == false`.
- `test_dhan_broker_name`: asserts `name() == "dhan"`.
(No HTTP calls in unit tests — just construction and trait-method tests.)

Run `cargo test --lib` after. All 220+ prior tests must still pass.

---

## Prompt 2 — Immutable Trade Log (append-only JSONL)

### Context

Phase 7 requires an immutable, tamper-evident record of every live order execution. This is separate from the strategy registry (`strategies.json`) and from the in-memory `PaperBroker` trade history. It must survive app restarts and must never be truncated by the app.

### Task

**A. `src/live/trade_log.rs`** — new file. Implement:

```rust
/// One entry in the immutable live trade log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TradeLogEntry {
    pub id: String,               // uuid v4
    pub timestamp: String,        // RFC 3339
    pub strategy_id: String,
    pub strategy_name: String,
    pub symbol: String,
    pub side: String,             // "BUY" | "SELL"
    pub quantity: i64,
    pub price: f64,               // fill price (from OrderResult, 0.0 if unavailable)
    pub order_id: String,         // broker order id
    pub order_status: String,     // "TRADED" | "REJECTED" | etc.
    pub mode: String,             // "live" always in this log
    pub rule_id: String,          // which engine rule triggered this
    pub notes: String,            // empty string normally; "stop_loss" | "take_profit" | "risk_breach" if applicable
}
```

```rust
/// Append-only writer. The file is JSONL: one JSON object per line, never truncated.
pub struct TradeLog {
    path: PathBuf,
    file: Mutex<std::fs::File>,
}

impl TradeLog {
    /// Open or create the log at `path`. Creates parent dirs.
    pub fn open(path: PathBuf) -> Result<Self, std::io::Error> { ... }

    /// Append one entry. Takes the file lock, writes one JSON line + '\n', flushes.
    pub fn append(&self, entry: TradeLogEntry) -> Result<(), std::io::Error> { ... }

    /// Read all entries from disk (for the IPC get_trade_log command).
    /// Skips malformed lines with an eprintln! warning.
    pub fn read_all(path: &PathBuf) -> Result<Vec<TradeLogEntry>, std::io::Error> { ... }
}
```

Rules:
- `open` opens with `OpenOptions::new().create(true).append(true)` — never truncate.
- `append` serializes with `serde_json::to_string` (not pretty), writes `"{json}\n"`, calls `flush()`. Lock is held for the duration of the write only.
- `read_all` opens for reading, iterates lines, `serde_json::from_str` each, collects valid entries. Skips blank lines silently. Skips malformed lines with `eprintln!("trade_log: skipping malformed line: {line}")`.
- `TradeLog` must be `Send + Sync` (the `Mutex<File>` guarantees this).

**B. `src/live/mod.rs`** — new file. `pub mod trade_log;` (more submodules will be added in Prompt 3).

**C. `src/lib.rs`** — add `pub mod live;`.

**D. `src/commands/state.rs`** — add `pub trade_log: Arc<TradeLog>` to `AppState`. The path is `app_data_dir/trade_log.jsonl`. Initialize it in the Tauri `setup` closure (Prompt 5 wires this).

**E. `src/commands/live.rs`** — new file. Implement:

```rust
/// Returns all entries from the trade log, newest first.
pub async fn get_trade_log(state: State<'_, AppState>) -> Result<Vec<TradeLogEntry>, String> {
    TradeLog::read_all(&state.trade_log_path)
        .map(|mut v| { v.reverse(); v })
        .map_err(|e| e.to_string())
}
```

Add `pub trade_log_path: PathBuf` to `AppState` (alongside `Arc<TradeLog>`) so `get_trade_log` can call `read_all` without borrowing the file handle.

**F. `src/commands/mod.rs`** — add `pub mod live;`.

**G. Unit tests** in `src/live/trade_log.rs`:
- `test_append_and_read`: create a temp-file `TradeLog`, append two entries, call `read_all`, assert both are present and fields match.
- `test_open_is_append_only`: open the same file twice, append one entry each time, call `read_all`, assert 2 entries total (not 1).
- `test_read_skips_malformed`: write a valid JSON line, a blank line, and a malformed line to the file manually, call `read_all`, assert only 1 entry returned.

Run `cargo test --lib` after. All prior tests must still pass.

---

## Prompt 3 — Live Session Manager

### Context

The live session manager owns the lifecycle of a running live strategy: it holds the engine, the Dhan broker, the candle assembler (1-min candles assembled from ticks), and the tick subscription. It is intentionally single-session — only one live strategy runs at a time in Phase 7.

`src/feed/manager.rs` already handles WebSocket tick subscriptions and fan-out. The feed manager is in `DataState.feed`. Tick fan-out delivers `Tick { symbol, price, volume, timestamp }` to subscribers via a channel.

The `StrategyEngine` `on_candle` method takes `&[Candle]` (a growing slice — the engine's `BoundedWindowProvider` does the windowing). The live path must assemble 1-minute candles from ticks and call `on_candle` at candle close (when a new minute starts).

### Task

**A. `src/live/candle_assembler.rs`** — new file. Implement a per-symbol 1-minute candle assembler:

```rust
pub struct CandleAssembler {
    symbol: String,
    current_minute: Option<i64>,   // unix timestamp of current minute (truncated to minute boundary)
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
}

impl CandleAssembler {
    pub fn new(symbol: String) -> Self { ... }

    /// Feed one tick. Returns Some(completed_candle) when a new minute starts
    /// (i.e., the tick's minute differs from the current open minute).
    /// Returns None while still assembling the current minute.
    pub fn feed(&mut self, tick: &Tick) -> Option<Candle> { ... }
}
```

Logic:
- `tick.timestamp` is a Unix timestamp in milliseconds. Truncate to minute boundary: `tick.timestamp / 60_000 * 60_000`.
- If `current_minute` is `None`, start the first candle; return `None`.
- If the tick's minute == `current_minute`, update `high`, `low`, `close`, accumulate `volume`; return `None`.
- If the tick's minute != `current_minute`, finalize the previous candle as `Candle { open, high, low, close, volume, timestamp: current_minute }`, then start a new candle with the current tick; return `Some(completed_candle)`.
- The very first tick of a new minute becomes `open = high = low = close = tick.price`, `volume = tick.volume`.

**B. `src/live/session.rs`** — new file. Implement:

```rust
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
    pub broker: Arc<DhanBroker>,
    pub trade_log: Arc<TradeLog>,
    pub candle_history: Arc<Mutex<Vec<Candle>>>,  // rolling candle buffer, newest last
    pub start_time: chrono::DateTime<chrono::Utc>,
    cancel: CancellationToken,
}

impl LiveSession {
    /// Construct and immediately start the tick-listening task.
    pub async fn start(
        strategy_id: String,
        strategy_name: String,
        symbol: String,
        strategy_node: StrategyNode,
        broker: Arc<DhanBroker>,
        feed: Arc<FeedManager>,
        trade_log: Arc<TradeLog>,
        event_bus: Arc<EventBus>,
        initial_candles: Vec<Candle>,   // seed from recent OHLCV fetch (last 500 1-min candles)
    ) -> Result<Arc<Self>, String> { ... }

    pub fn pause(&self) { ... }   // sets status to Paused; tick loop checks and skips on_candle
    pub fn resume(&self) { ... }  // sets status back to Running
    pub async fn stop(&self) { ... }  // cancels the CancellationToken; waits for task exit
    pub fn status(&self) -> SessionStatus { ... }
}
```

The tick-listening task (spawned in `start`):
1. Subscribe to ticks for `symbol` via `feed.subscribe(symbol)`. This gives a `tokio::sync::broadcast::Receiver<Tick>`.
2. Construct a `CandleAssembler` for the symbol.
3. Loop: `recv()` ticks. On `CancellationToken` cancelled, break. On `SessionStatus::Paused`, receive but skip `on_candle`. On a completed candle from the assembler, push to `candle_history`, then call `engine.lock().on_candle(&candle_history.lock())`. Collect the returned `Vec<LogEntry>`. For each `LogEntry` where `kind == LogEntryKind::OrderExecuted`, construct a `TradeLogEntry` and call `trade_log.append(...)`.
4. On any `on_candle` error: log to stderr, set `status` to `Failed(err)`, break.
5. On task exit (cancel or fail), set `status` to `Stopped` (if not already `Failed`).

`candle_history` is pre-seeded from `initial_candles` (the last 500 1-min candles fetched at session start). The engine's `BoundedWindowProvider` handles the windowing — just keep appending to `candle_history` and pass the full slice each call.

**C. `src/live/mod.rs`** — add `pub mod candle_assembler; pub mod session;`.

**D. `src/commands/state.rs`** — add `pub live_session: Arc<Mutex<Option<Arc<LiveSession>>>>` to `AppState`.

**E. Unit tests** in `src/live/candle_assembler.rs`:
- `test_first_tick_no_candle`: feed one tick, assert `None` returned.
- `test_same_minute_no_candle`: feed three ticks in the same minute, assert `None` each time.
- `test_new_minute_returns_candle`: feed ticks across a minute boundary, assert `Some(candle)` on the first tick of the new minute, assert OHLCV values of the completed candle are correct.
- `test_high_low_tracked`: feed three ticks in one minute with prices 100, 105, 98, assert `high == 105` and `low == 98`.

Run `cargo test --lib` after.

---

## Prompt 4 — Safety Gate Layer (LiveGuard)

### Context

This is the most important prompt. No live order must ever be placed unless all gates pass. The gate runs before `LiveSession::start` and is non-bypassable.

Gates (all must pass; first failure aborts with a descriptive error):

1. **Paper-default guard**: `StrategyMode` must be explicitly `Live` — paper is the default, the caller must have intentionally set live mode.
2. **Broker reachability**: call `client.get_positions()` (a lightweight authenticated GET); if it fails, abort with the error message.
3. **Symbol in map**: the strategy's symbol must be present in `SymbolMap`. Abort if missing.
4. **Market hours guard**: Indian market is open Monday–Friday 09:15–15:30 IST. Reject attempts to start a live session outside these hours. Use the system clock. The check is advisory (not a hard block for paper), but for live mode it is a hard gate. Return error: `"market is closed; live trading is only allowed 09:15–15:30 IST on weekdays"`.
5. **Risk config required**: the strategy must have at least one risk declaration (`RiskConfig` present with at least one of `max_orders`, `max_positions`, or `max_daily_loss` set). Abort with: `"live strategies must declare at least one RISK control (MAX_ORDERS, MAX_POSITIONS, or MAX_DAILY_LOSS)"`.
6. **Max daily loss required**: specifically, `RiskConfig.max_daily_loss` must be `Some(_)`. Abort with: `"live strategies must declare RISK MAX_DAILY_LOSS"`. This is non-negotiable — it is the only hard financial safety net the engine enforces per-candle.
7. **First-live acknowledgment**: read a flag from `<app_data>/live_ack.json`. If the file does not exist or `{ "acknowledged": false }`, this gate does NOT abort — instead it returns a special `LiveGuardResult::RequiresAcknowledgment` variant that the IPC layer translates into a special error code the UI interprets to show the first-live warning modal. Once the user acknowledges (via `acknowledge_live_trading` IPC command, which writes `{ "acknowledged": true, "timestamp": "<RFC 3339>" }` to the file), subsequent starts skip this modal.
8. **Two-step confirmation token**: the IPC flow for starting a live session is two-step. `request_live_start(strategy_id)` runs gates 1–7 and, if all pass, issues a short-lived token (`uuid v4`, valid for 30 seconds, stored in `AppState.pending_live_token: Arc<Mutex<Option<PendingLiveToken>>>`). `confirm_live_start(strategy_id, token)` validates the token (correct, not expired, correct strategy_id) and only then calls `LiveSession::start`. If the token is wrong, expired, or mismatched, return an error — force the user through `request_live_start` again.

```rust
/// src/live/guard.rs

pub struct LiveGuard {
    pub client: Arc<DhanClient>,
    pub symbol_map: Arc<parking_lot::RwLock<SymbolMap>>,
    pub ack_path: PathBuf,
}

pub enum LiveGuardResult {
    Ok,
    RequiresAcknowledgment,
}

pub struct PendingLiveToken {
    pub token: String,
    pub strategy_id: String,
    pub expires_at: std::time::Instant,
}

impl LiveGuard {
    pub async fn run_preflight(
        &self,
        symbol: &str,
        strategy_node: &StrategyNode,
    ) -> Result<LiveGuardResult, String> { ... }

    pub fn issue_token(strategy_id: &str) -> PendingLiveToken { ... }

    pub fn validate_token(
        pending: &Option<PendingLiveToken>,
        strategy_id: &str,
        token: &str,
    ) -> Result<(), String> { ... }
}
```

`run_preflight` runs gates 1–7 in order (gate numbering above). Gate 8 (token) is enforced by the IPC layer, not inside `run_preflight`.

`issue_token` generates a `uuid::Uuid::new_v4()` token with a 30-second TTL (`Instant::now() + Duration::from_secs(30)`).

`validate_token` checks: `pending.is_some()`, `token == pending.token`, `strategy_id == pending.strategy_id`, `Instant::now() < pending.expires_at`. Any failure → `Err("invalid or expired confirmation token")`.

**B. `src/commands/live.rs`** — add (alongside `get_trade_log`):

```rust
/// Gate step 1. Runs preflight and issues a 30s token on success.
/// Returns { "token": "...", "requires_ack": false } on success,
/// or { "token": "", "requires_ack": true } when ack is needed,
/// or Err(message) on a hard gate failure.
pub async fn request_live_start(
    state: State<'_, AppState>,
    strategy_id: String,
) -> Result<RequestLiveStartResult, String> { ... }

/// Gate step 2. Validates the token then launches the live session.
pub async fn confirm_live_start(
    state: State<'_, AppState>,
    strategy_id: String,
    token: String,
) -> Result<(), String> { ... }

/// Write { "acknowledged": true } to live_ack.json.
pub async fn acknowledge_live_trading(
    state: State<'_, AppState>,
) -> Result<(), String> { ... }
```

`confirm_live_start`:
1. Validate token via `LiveGuard::validate_token`.
2. Clear `pending_live_token` immediately (one-use token).
3. Look up the strategy from `StrategyRegistry` by `strategy_id`. Parse and validate its DSL.
4. Fetch last 500 1-min candles for the symbol via `DataState.broker.get_ohlcv(...)`. Tolerate failure (use empty vec with a stderr warning — the engine will warm up over time).
5. Call `LiveSession::start(...)`.
6. Store the session in `AppState.live_session`.

**C. `src/live/mod.rs`** — add `pub mod guard;`.

**D. `src/commands/state.rs`** — add `pub pending_live_token: Arc<Mutex<Option<PendingLiveToken>>>` and `pub live_guard: Arc<LiveGuard>` and `pub ack_path: PathBuf` to `AppState`.

**E. Unit tests** in `src/live/guard.rs`:
- `test_token_expires`: issue a token with TTL 0 (set `expires_at = Instant::now()`), call `validate_token` after 1ms, assert `Err`.
- `test_token_wrong_id`: issue a token for `"strat-1"`, validate with `"strat-2"`, assert `Err`.
- `test_token_wrong_token`: validate with the wrong token string, assert `Err`.
- `test_token_valid`: issue a token with a 30s TTL, validate immediately with correct fields, assert `Ok`.
- `test_market_hours_check` (pure function test): test the market hours predicate at 09:14 IST Monday (should fail), 09:15 IST Monday (should pass), 15:30 IST Monday (should pass), 15:31 IST Monday (should fail), 10:00 IST Saturday (should fail). Extract the predicate as a `pub fn is_market_open(dt: chrono::DateTime<chrono::FixedOffset>) -> bool` so it is testable without a real clock.

Run `cargo test --lib` after.

---

## Prompt 5 — Tauri IPC Commands + AppState Wiring + Event Bus for Live

### Context

All library code is now in place. This prompt wires everything into the Tauri shell: `AppState`, `main.rs` `setup`, the `generate_handler!` list, and the event-bus hook that was left as a TODO in earlier phases.

### Task

**A. `src/commands/state.rs`** — finalize `AppState` to include all Phase 7 fields (ensure no duplicates with earlier prompts):

```rust
pub struct AppState {
    // Existing:
    pub data: DataState,                                    // broker + feed
    pub registry: Arc<StrategyRegistry>,
    pub plugin_registry: Arc<PluginRegistry>,
    pub ui_rx: tokio::sync::broadcast::Receiver<UiMessage>,

    // Phase 7 additions:
    pub trade_log: Arc<TradeLog>,
    pub trade_log_path: PathBuf,
    pub live_session: Arc<Mutex<Option<Arc<LiveSession>>>>,
    pub live_guard: Arc<LiveGuard>,
    pub pending_live_token: Arc<Mutex<Option<PendingLiveToken>>>,
    pub ack_path: PathBuf,
}
```

**B. `src-tauri/src/main.rs`** — inside the `setup` closure, after all existing setup code, add:

```
// Phase 7 — Trade log
let trade_log_path = app_data_dir.join("trade_log.jsonl");
let trade_log = Arc::new(TradeLog::open(trade_log_path.clone())
    .expect("failed to open trade log"));

// Phase 7 — Live guard
let ack_path = app_data_dir.join("live_ack.json");
let live_guard = Arc::new(LiveGuard {
    client: dhan_client.clone(),   // the same Arc<DhanClient> already constructed for DataState
    symbol_map: symbol_map.clone(),
    ack_path: ack_path.clone(),
});

// Phase 7 — Pending token + live session slot
let pending_live_token: Arc<Mutex<Option<PendingLiveToken>>> = Arc::new(Mutex::new(None));
let live_session: Arc<Mutex<Option<Arc<LiveSession>>>> = Arc::new(Mutex::new(None));
```

Include all Phase 7 fields when constructing `AppState`.

**C. Event bus — live engine wiring.** The TODO comment in `main.rs` says "stage 9 hook: wire event bus to live engine". Implement this:

When `confirm_live_start` calls `LiveSession::start`, pass the shared `event_bus: Arc<EventBus>` from `AppState` (it was built in the plugin setup closure). Inside `LiveSession::start`, assign it to the `StrategyEngine` via `engine.set_event_bus(Some(event_bus))`. The `StrategyEngine` already has `event_bus: Option<Arc<EventBus>>` and publishes `RuleFired`, `TradeExecuted`, `CandleProcessed` when `Some`. This means live runs get plugin event callbacks; paper/backtest still get `None`.

**D. Plugin `Execution` capability wiring.** Currently `NoopExecutionApi` is used everywhere. When a live session is active, the `PluginHost`'s `Execution` slot should be backed by the live session's `DhanBroker`. Update the `HostFactory` closure in `main.rs`: add a `live_session: Arc<Mutex<Option<Arc<LiveSession>>>>` capture. Inside the factory, check if a live session is active; if so, clone `session.broker` and pass an `Arc<DhanBrokerExecutionApi>` as the execution API. If not active, fall back to `NoopExecutionApi`.

```rust
// src/plugin/api/execution.rs — add alongside NoopExecutionApi:
pub struct DhanBrokerExecutionApi {
    broker: Arc<DhanBroker>,
}

impl ExecutionApi for DhanBrokerExecutionApi {
    fn submit_order(&self, symbol: &str, side: &str, qty: i64) -> Result<String, PluginError> {
        // Build an Order and call broker.execute via block_on.
        // Returns the order_id string on success.
    }
    fn cancel_order(&self, order_id: &str) -> Result<(), PluginError> {
        // Not supported in Phase 7 — return PluginError::ApiError("cancel not supported".into())
    }
    fn positions(&self) -> Result<Vec<PluginPosition>, PluginError> {
        // Call broker.get_positions via block_on, map to PluginPosition.
    }
}
```

**E. `src/commands/live.rs`** — add the remaining IPC command bodies:

```rust
/// Pause the running live session (no new on_candle calls, but tick loop stays alive).
pub async fn pause_live_strategy(state: State<'_, AppState>) -> Result<(), String> { ... }

/// Resume a paused live session.
pub async fn resume_live_strategy(state: State<'_, AppState>) -> Result<(), String> { ... }

/// Stop and tear down the live session. Positions are NOT automatically closed —
/// the user must manually manage open positions in their broker app.
/// Returns a warning string if positions were open at stop time.
pub async fn stop_live_strategy(state: State<'_, AppState>) -> Result<StopResult, String> { ... }

/// Snapshot of the current live session for the UI.
pub async fn get_live_status(state: State<'_, AppState>) -> Result<Option<LiveStatusWire>, String> { ... }
```

`StopResult`:
```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StopResult {
    pub stopped: bool,
    pub open_positions_warning: Option<String>,  // e.g. "WARNING: 5 open positions remain in NIFTY — close them manually in your broker app"
}
```

`LiveStatusWire`:
```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveStatusWire {
    pub strategy_id: String,
    pub strategy_name: String,
    pub symbol: String,
    pub status: String,           // "Starting" | "Running" | "Paused" | "Stopped" | "Failed"
    pub fail_reason: Option<String>,
    pub start_time: String,       // RFC 3339
    pub position_count: i64,
    pub realized_loss: f64,
}
```

**F. `src-tauri/src/main.rs`** — add `#[tauri::command]` wrappers and register in `generate_handler!`:

New commands: `request_live_start`, `confirm_live_start`, `acknowledge_live_trading`, `pause_live_strategy`, `resume_live_strategy`, `stop_live_strategy`, `get_live_status`, `get_trade_log`.

Pattern: one-line wrapper delegating to `commands::live::*`.

**G. `src/types/tauri.ts`** (frontend) — add TS wrappers for all 8 new commands:

```typescript
export async function requestLiveStart(strategyId: string): Promise<RequestLiveStartResult> { ... }
export async function confirmLiveStart(strategyId: string, token: string): Promise<void> { ... }
export async function acknowledgeLiveTrading(): Promise<void> { ... }
export async function pauseLiveStrategy(): Promise<void> { ... }
export async function resumeLiveStrategy(): Promise<void> { ... }
export async function stopLiveStrategy(): Promise<StopResult> { ... }
export async function getLiveStatus(): Promise<LiveStatusWire | null> { ... }
export async function getTradeLog(): Promise<TradeLogEntry[]> { ... }
```

Add the corresponding TS interfaces to `src/types/strategy.ts` or a new `src/types/live.ts`:

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
```

Run `cargo test --lib` and `npm run build` after. All prior tests must pass. TypeScript must type-check cleanly.

---

## Prompt 6 — UI: Live Trading Screen + Confirmation Flow

### Context

The app already has: `Builder`, `Strategies`, `Plugins`, `Settings` screens. The sidebar has four nav items. A fifth — `Live` — will be added. The screen shows live session status, active positions, P&L, and the trade log table.

Design language: dark green-black palette, lime-green (`--text-green`) accents, Cascadia Code for monospace data, CSS Modules, no external UI component libraries, all design tokens via CSS custom properties. Match the visual density of `StrategiesScreen` (card-based layout).

### Task

**A. `src/screens/Live/LiveScreen.tsx` + `LiveScreen.module.css`** — new files.

The screen has four sections laid out vertically:

**Section 1 — Status Card** (always visible, even when no session is running):
- Header: "Live Trading" with a pulsing dot indicator (CSS animation: `border-radius: 50%`, `background: var(--text-green)`, 1s ease-in-out infinite alternate opacity 0.3→1.0) shown only when status is `Running`.
- Body when no session: centered text "No live strategy running." with a subdued `--text-dim` color.
- Body when session exists: strategy name, symbol chip, status badge (color-coded: Running = `--text-green`, Paused = yellow literal `#c8a84b`, Failed = `#c85a54`, Stopped = `--text-dim`), start time formatted as `"Started HH:MM:SS"`, realized loss formatted as `"Session loss: ₹{n}"` (red if > 0, `--text-dim` if 0).

**Section 2 — Positions Card** (shown only when session is Running or Paused):
- Header: "Open Positions"
- Table columns: Symbol | Qty | Avg Price | Unrealized P&L
- Fetches positions via `getLiveStatus()` polled every 5 seconds (use `useEffect` + `setInterval`, clear on unmount). Display `positionCount` in the header as a badge. If 0 positions, show "No open positions."
- P&L column: green if positive, red if negative.

**Section 3 — Controls Row** (shown only when a session exists):
- Three `Button` components side by side:
  - **Pause** (variant `ghost`): calls `pauseLiveStrategy()`. Disabled when status is not `Running`.
  - **Resume** (variant `ghost`): calls `resumeLiveStrategy()`. Disabled when status is not `Paused`.
  - **Stop** (variant `ghost`, red styling via inline CSS `color: #c85a54; border-color: #c85a54`): calls `stopLiveStrategy()`. On success, if `openPositionsWarning` is non-null, show it as a toast. Always shown; disabled while `Starting`.

**Section 4 — Trade Log Card**:
- Header: "Trade Log" with a count badge.
- Table columns: Time | Strategy | Symbol | Side | Qty | Price | Order ID | Notes
- `side` column: "BUY" in `--text-green`, "SELL" in `#c85a54`.
- `notes` column: dim if empty, red if `"stop_loss"` or `"risk_breach"`, yellow if `"take_profit"`.
- Fetches via `getTradeLog()` on mount. Refresh button (ghost, "↻ Refresh") re-fetches.
- If no entries: "No live trades recorded yet." in `--text-dim`.
- Table is scrollable, `max-height: 280px`.

**B. Confirmation Modal** — `src/components/LiveConfirmModal/LiveConfirmModal.tsx` + `LiveConfirmModal.module.css`:

This modal is shown from `StrategiesScreen` when the user clicks "Go Live" on a deployed strategy card. It is a two-step flow:

**Step 1 — Pre-flight** (shown immediately when the modal opens):
- Title: "Start Live Trading"
- Body: a list of all pre-flight checks that will run, as static text. Includes: "Market hours check", "Broker connectivity check", "Symbol map check", "Risk controls check".
- One large warning box: `background: rgba(200, 90, 84, 0.1); border: 1px solid #c85a54; border-radius: 6px; padding: 12px;` containing: "⚠ Live trading places real orders with real money. Losses may exceed your configured limits in fast markets. Make sure you have reviewed your strategy's backtest and understand the risks."
- Two buttons: "Cancel" (ghost) and "Run Checks" (primary). Clicking "Run Checks" calls `requestLiveStart(strategyId)` and transitions to Step 2.
- If `requestLiveStart` returns an error, show the error string in red below the buttons and stay on Step 1.

**First-live acknowledgment sub-step** (inserted between Step 1 and Step 2 if `requiresAck == true`):
- Shows a full-screen-modal (within the modal) warning about first live trade.
- Title: "First Live Trade Warning"
- Body: "You are about to place your first live order on AlgoMLN. Once confirmed, real orders will be sent to Dhan on your behalf. Paper trading is always available and recommended for new strategies. Do you understand and accept the risks?"
- Two buttons: "Cancel" and "I Understand — Proceed" (primary). Clicking "I Understand" calls `acknowledgeLiveTrading()`, then transitions to Step 2 with the already-issued token.

**Step 2 — Confirm** (shown after pre-flight passes):
- Title: "Confirm Live Start"
- Body: the strategy name, symbol, a reminder that stop will NOT auto-close positions.
- Two buttons: "Cancel" (ghost) and "Confirm — Go Live" (primary, with a 3-second countdown before it becomes clickable: button shows "Wait (3)…", "Wait (2)…", "Wait (1)…", then "Confirm — Go Live"). This prevents accidental clicks.
- On click: calls `confirmLiveStart(strategyId, token)`. On success, closes the modal, navigates to the `Live` screen. On error, shows the error and returns to Step 1.

**C. `src/screens/Strategies/StrategiesScreen.tsx`** — add a "Go Live" button to each `StrategyCard`. Only shown for strategies with `mode == 'live'`. Clicking it opens the `LiveConfirmModal` with the strategy's id. After the flow completes, bump the strategies refresh key and navigate to the Live screen.

**D. `src/components/Sidebar/Sidebar.tsx`** — add a fifth nav item: `{ id: 'live', label: 'Live', icon: '◉' }` between `Plugins` and `Settings`.

**E. `src/App.tsx`** — add `'live'` to the `Screen` type, add `LiveScreen` to the render switch, add `liveModal` to the modal state (or track the live confirm modal with `liveConfirmStrategyId: string | null`). Pass the navigate-to-live callback into `StrategiesScreen`.

**F. Browser fallback** — `getLiveStatus()` and `getTradeLog()` must check `isTauri()` and return `null` / `[]` respectively when running in the browser. `requestLiveStart` and `confirmLiveStart` should return a descriptive `Err("live trading is not available in the browser")` via a rejected Promise.

**G. `src/hooks/useLiveStatus.ts`** — new hook:

```typescript
export function useLiveStatus(pollIntervalMs = 5000) {
    const [status, setStatus] = useState<LiveStatusWire | null>(null);
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        // fetch once on mount
        // then setInterval for subsequent polls
        // clear interval on unmount
    }, [pollIntervalMs]);

    return { status, error, refresh: () => { /* manual refetch */ } };
}
```

Use this hook in `LiveScreen` for the status card and positions card.

Run `npm run build` after. TypeScript must type-check cleanly. Verify `npm run dev` works in the browser (fallback paths exercised).

---

## Post-Phase 7 Audit Checklist

After running all 6 prompts, verify the following manually before doing any live trading:

- [ ] `cargo test --workspace` passes with 0 failures.
- [ ] `npm run build` exits 0 with no TS errors.
- [ ] In the Strategies screen, a live-mode strategy shows the "Go Live" button.
- [ ] Clicking "Go Live" outside market hours shows the market-hours error on Step 1.
- [ ] Clicking "Go Live" on a strategy with no `RISK MAX_DAILY_LOSS` declaration shows the risk error.
- [ ] First-time clicking "Go Live" shows the First Live Trade Warning modal.
- [ ] The 3-second countdown on Step 2 is functional.
- [ ] The Live screen shows "No live strategy running." when no session exists.
- [ ] Starting a session (paper test or dry-run against Dhan sandbox) populates the status card.
- [ ] Stopping a session shows the open-positions warning if positions remain.
- [ ] The Trade Log table populates after a trade is executed.
- [ ] The `trade_log.jsonl` file in `%APPDATA%\com.algomln.app\` grows on every live order and is never truncated.
- [ ] Plugin `Execution` capability (`submit_order`) routes to `DhanBrokerExecutionApi` when a session is active, `NoopExecutionApi` when idle.

---

## MD File Updates

After all prompts are run and verified, update the project docs:

**`README.md`** — flip Phase 7 to `✅` in the status table. Add a Phase 7 section under the roadmap.

**`CLAUDE.md`** — add invariants:
- **14. Live sessions are single-instance.** Only one `LiveSession` may be active at a time. `AppState.live_session` holds `Mutex<Option<Arc<LiveSession>>>`. `confirm_live_start` returns an error if a session is already active. Stopping a session clears the slot.
- **15. Trade log is append-only.** `TradeLog::open` uses `OpenOptions::append(true)`. The file at `<app_data>/trade_log.jsonl` is never truncated by the application. Do not add any truncate/rotate logic to `TradeLog`.
- **16. Live engine has the event bus; backtests do not.** Invariant 8 is now verified at the session level: `LiveSession::start` always calls `engine.set_event_bus(Some(bus))`. `run_backtest_internal` always passes `None`. Do not relax either side.
- **17. Pre-flight gates are non-bypassable.** All 7 gates in `LiveGuard::run_preflight` must pass before a token is issued. Do not add `cfg(test)` bypasses or feature flags that skip gates in production builds. Test the guard logic with pure-function unit tests instead.

**`ARCHITECTURE.md`** — add `src/live/` subtree, `src/commands/live.rs`, `src/screens/Live/`, `src/components/LiveConfirmModal/`, `src/hooks/useLiveStatus.ts`, `src/types/live.ts` to the file tree and lookup tables.

**`BACKEND.md`** — add a "Live Trading" section covering `DhanBroker`, `TradeLog`, `LiveSession`, `LiveGuard`, the two-step IPC flow, and the event-bus wiring.

**`FRONTEND.md`** — add `LiveScreen`, `LiveConfirmModal`, `useLiveStatus` to the screens section and hooks section.
