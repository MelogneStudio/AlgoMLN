# AlgoMLN — Phase 7, Prompt 4 (Fixed): Safety Gate Layer (`LiveGuard`)

Run this prompt in Claude Code / Cursor / Windsurf against the existing repo.
This is the fixed version of Phase 7 Prompt 4, incorporating corrections
identified in review.

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

Prompts 1–3 of Phase 7 have been implemented, plus a follow-up fix pack.
You can assume the following code exists in the repo:

- `src/broker/dhan/rest.rs` — `DhanClient` with `place_order`,
  `get_positions`, and OHLCV methods. `place_order` includes a
  `correlationId` and does not auto-retry.
- `src/strategy/execution/dhan.rs` — `DhanBroker` implementing
  `ExecutionTarget`. It owns an `Arc<TradeLog>` and a
  `SessionContext` slot; `execute_with_meta(order, rule_id, notes)` is
  the primary entry point. `realized_loss()` returns a non-negative
  magnitude sourced from a background positions refresh; `is_stale()`
  signals refresh failure.
- `src/live/trade_log.rs` — `TradeLog` (append-only JSONL) with
  `TradeLogEntry`.
- `src/live/candle_assembler.rs` — 1-minute `CandleAssembler`.
- `src/live/session.rs` — `LiveSession` with tick loop, pause semantics
  where pause blocks entries but keeps exits/risk running, bounded
  `candle_history`, and Tauri event emission on `Failed`.
- `src/models/` — `Order`, `OrderResult`, `Position`, `OrderStatus`
  (with `is_terminal()` and `is_fill()`), `StrategyMode` (`Paper`,
  `Live`), `RiskConfig` (`max_orders`, `max_positions`,
  `max_daily_loss`), `StrategyNode` (parsed DSL), `SymbolMap` (with
  per-entry `segment: Segment`).
- `src/commands/state.rs` — `AppState` with the fields added in
  Prompts 1–3 (`trade_log`, `trade_log_path`, `live_session`, `data`,
  `registry`, `plugin_registry`, `ui_rx`).

If any of the above does not exist as described, **stop and ask** before
writing code. Do not paper over gaps.

---

## Task

Build the safety gate layer. No live order may ever be placed unless every
gate below passes.

### The gates (all must pass; first failure aborts with a descriptive error)

1. **Paper-default guard.** `StrategyMode` on the strategy must be
   explicitly `Live`. Paper is the default; a caller that reaches this
   code must have intentionally opted in.
2. **Broker reachability.** Call `client.get_positions()`. If it fails,
   abort with the returned error string.
3. **Symbol in map.** The strategy's symbol must resolve in `SymbolMap`.
4. **Segment guard.** The resolved `SymbolMap` entry must have
   `segment == Segment::NseEq`. Abort with:
   `"Phase 7 only supports NSE equity intraday trading; symbol {sym} is
   {seg:?}"`. (This duplicates the check inside `DhanBroker::place_order`
   from the fix pack. Defence in depth — keep both.)
5. **Market hours guard.** Indian equity market opens 09:15 IST and closes
   15:30 IST, Monday–Friday, excluding NSE holidays. This is a hard gate
   for live mode. Return:
   `"market is closed; live trading is only allowed 09:15–15:30 IST on
   NSE trading days"`.
6. **Risk config required.** The strategy must have a `RiskConfig` with
   at least one of `max_orders`, `max_positions`, `max_daily_loss` set.
   Abort: `"live strategies must declare at least one RISK control
   (MAX_ORDERS, MAX_POSITIONS, or MAX_DAILY_LOSS)"`.
7. **Max daily loss required.** Specifically,
   `RiskConfig.max_daily_loss` must be `Some(_)`. Abort:
   `"live strategies must declare RISK MAX_DAILY_LOSS"`. Non-negotiable —
   it is the only hard financial safety net in the engine.
8. **Broker not stale.** Check `broker.is_stale() == false`. If the
   broker's positions-refresh background task has been failing (see
   `DhanBroker` in Fix 1), the loss tracker is unreliable and starting a
   session is unsafe. Abort:
   `"broker realized-loss tracking is stale; check broker connectivity
   and retry"`.
9. **First-live acknowledgment.** Read `<app_data>/live_ack.json`. If
   the file doesn't exist or contains `{ "acknowledged": false }`, this
   gate does not abort. It returns
   `LiveGuardResult::RequiresAcknowledgment { token }` — see
   "**Ack flow correction**" below.

Gate order matters: cheap checks (paper-default, risk config) first, then
symbol lookup, then network calls (broker reachability). Ack is last.

### Ack flow correction (important)

The original spec said `request_live_start` returns
`{ token: "", requires_ack: true }` when ack is needed, and the UI would
call `confirmLiveStart` with the "already-issued token" after
acknowledgment. There is no such token — this would fail every first-ever
live start with an empty-token error.

**Correct flow:**

- If gates 1–8 pass and ack is not required: issue a token, return
  `{ token, requires_ack: false }`.
- If gates 1–8 pass but ack is required: **still issue the token**, return
  `{ token, requires_ack: true }`. The UI shows the ack modal, calls
  `acknowledge_live_trading` (which just writes the ack file), and then
  proceeds to `confirmLiveStart(strategy_id, token)` with the token
  received in step 1.
- On gate 1–8 failure: return an `Err(String)` describing which gate
  failed. No token is issued.

This way, `acknowledge_live_trading` never needs to know about tokens; it
is a pure "user checked the box" operation. The token lifecycle stays
entirely inside `request_live_start` / `confirm_live_start`.

Ack expiry: the ack file is a one-time consent (session-of-app is not
needed). Once written, it stays.

### Token semantics

- 90-second TTL (previously 30s was too tight against a 3-second countdown
  and a reader-of-warnings pause).
- Single use: `confirm_live_start` clears the pending token immediately
  after validating it, whether the subsequent session start succeeds or fails.
- Bound to strategy id: attempting to confirm with the same token for a
  different strategy id fails.
- Only one token outstanding at a time: calling `request_live_start` again
  while a pending token exists overwrites it (the newer request wins,
  older token is invalidated).

---

## Files to create/edit

### A. `src/live/guard.rs` — new file

```rust
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use chrono::{DateTime, FixedOffset};
use parking_lot::RwLock;
use uuid::Uuid;

pub struct LiveGuard {
    pub client: Arc<DhanClient>,
    pub symbol_map: Arc<RwLock<SymbolMap>>,
    pub broker: Arc<DhanBroker>,   // for is_stale() check
    pub ack_path: PathBuf,
    pub holiday_calendar: Arc<NseHolidayCalendar>,
}

pub enum LiveGuardResult {
    Ok { token: PendingLiveToken },
    RequiresAcknowledgment { token: PendingLiveToken },
}

pub struct PendingLiveToken {
    pub token: String,
    pub strategy_id: String,
    pub expires_at: Instant,
}

impl LiveGuard {
    /// Runs gates 1–9 in order. Returns Ok(...) with a fresh token on
    /// success, or Err(msg) on the first gate failure. The token TTL is
    /// 90 seconds.
    pub async fn run_preflight(
        &self,
        symbol: &str,
        strategy_id: &str,
        strategy_node: &StrategyNode,
    ) -> Result<LiveGuardResult, String> { ... }

    pub fn issue_token(strategy_id: &str) -> PendingLiveToken {
        PendingLiveToken {
            token: Uuid::new_v4().to_string(),
            strategy_id: strategy_id.to_string(),
            expires_at: Instant::now() + Duration::from_secs(90),
        }
    }

    /// Consumes/validates. Returns Ok if the pending token matches
    /// strategy_id + token string and has not expired. Any failure →
    /// Err("invalid or expired confirmation token").
    pub fn validate_token(
        pending: &Option<PendingLiveToken>,
        strategy_id: &str,
        token: &str,
    ) -> Result<(), String> { ... }
}

/// Extract as a pure function so it is unit-testable without a real
/// system clock. Callers pass `chrono::Local::now().with_timezone(&IST)`.
pub fn is_market_open(
    dt: DateTime<FixedOffset>,
    holidays: &NseHolidayCalendar,
) -> bool { ... }

fn read_ack_file(path: &PathBuf) -> bool {
    // Returns true if the file exists and parses to
    // { "acknowledged": true }. Any read/parse error → false (safe default:
    // require ack again).
    ...
}
```

### B. `src/live/holidays.rs` — new file

Minimal NSE holiday calendar. The list changes annually; make it easy to
update.

```rust
use chrono::NaiveDate;

pub struct NseHolidayCalendar {
    holidays: Vec<NaiveDate>,
}

impl NseHolidayCalendar {
    pub fn new() -> Self {
        // Hardcode NSE trading holidays for the current + next calendar
        // year. Source: NSE circular. Keep this list short and current;
        // Phase 8 can replace it with a fetched source.
        Self {
            holidays: vec![
                // Populate with the current known list, formatted:
                // NaiveDate::from_ymd_opt(YYYY, MM, DD).unwrap(),
                // Do not fabricate — if you don't have the list, leave a
                // TODO comment and ask the user to supply the current
                // holiday list. See "Ask the user" at the bottom.
            ],
        }
    }

    pub fn is_holiday(&self, d: NaiveDate) -> bool {
        self.holidays.iter().any(|h| *h == d)
    }
}
```

**Muhurat sessions are ignored in Phase 7.** They are the only case where
the market is open outside the standard 09:15–15:30 window; they happen
once a year on Diwali. Document as a known limitation: users cannot run
live sessions during Muhurat trading in Phase 7. Phase 8 can add it.

### C. `src/commands/live.rs` — extend

Add to the existing file (which already has `get_trade_log` from
Prompt 2):

```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestLiveStartResult {
    pub token: String,
    pub requires_ack: bool,
}

pub async fn request_live_start(
    state: State<'_, AppState>,
    strategy_id: String,
) -> Result<RequestLiveStartResult, String> {
    // 1. Reject if a session is already active.
    if state.live_session.lock().is_some() {
        return Err("a live session is already active; stop it before starting another".into());
    }

    // 2. Look up strategy from registry.
    let strategy = state.registry.get(&strategy_id)
        .ok_or_else(|| format!("strategy not found: {strategy_id}"))?;
    let strategy_node = parse_and_validate_dsl(&strategy.dsl)
        .map_err(|e| e.to_string())?;

    // 3. Run preflight (gates 1–9).
    let result = state.live_guard
        .run_preflight(&strategy.symbol, &strategy_id, &strategy_node)
        .await?;

    // 4. Store the pending token (whichever variant).
    let (token, requires_ack) = match result {
        LiveGuardResult::Ok { token } => (token, false),
        LiveGuardResult::RequiresAcknowledgment { token } => (token, true),
    };
    let token_string = token.token.clone();
    *state.pending_live_token.lock() = Some(token);

    Ok(RequestLiveStartResult { token: token_string, requires_ack })
}

pub async fn confirm_live_start(
    state: State<'_, AppState>,
    strategy_id: String,
    token: String,
) -> Result<(), String> {
    // 1. Re-check "no session already active" atomically with the insert
    //    at the end. Take the live_session lock and hold it across the
    //    entire confirm — see note below on lock discipline.
    let mut session_slot = state.live_session.lock();
    if session_slot.is_some() {
        return Err("a live session is already active".into());
    }

    // 2. Validate + clear token.
    {
        let mut pending = state.pending_live_token.lock();
        LiveGuard::validate_token(&*pending, &strategy_id, &token)?;
        *pending = None;
    }

    // 3. Look up strategy, parse DSL (same as request_live_start).
    let strategy = state.registry.get(&strategy_id)
        .ok_or_else(|| format!("strategy not found: {strategy_id}"))?;
    let strategy_node = parse_and_validate_dsl(&strategy.dsl)
        .map_err(|e| e.to_string())?;

    // 4. Fetch seed candles. Tolerate failure with a stderr warning.
    let seed = match state.data.broker.get_ohlcv_intraday(&strategy.symbol, 500).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("confirm_live_start: seed OHLCV fetch failed: {e}; starting cold");
            vec![]
        }
    };

    // 5. Start the session.
    let session = LiveSession::start(
        strategy_id.clone(),
        strategy.name.clone(),
        strategy.symbol.clone(),
        strategy_node,
        state.data.dhan_broker.clone(),
        state.data.feed.clone(),
        state.trade_log.clone(),
        state.event_bus.clone(),
        seed,
        state.app_handle.clone(),   // for failure event emission
    ).await?;

    // 6. Insert into slot.
    *session_slot = Some(session);
    Ok(())
}

pub async fn acknowledge_live_trading(
    state: State<'_, AppState>,
) -> Result<(), String> {
    let payload = serde_json::json!({
        "acknowledged": true,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(&state.ack_path, serde_json::to_string_pretty(&payload).unwrap())
        .map_err(|e| format!("failed to write ack file: {e}"))?;
    Ok(())
}
```

**Lock discipline note:** the `live_session` lock is held across
`LiveSession::start`. If `start` is heavy or does its own awaits, use a
sentinel pattern instead: insert a `Reserved` marker in the slot, drop
the lock, do the work, then swap in the real `Arc<LiveSession>` (or
clear the marker on failure). Choose whichever gives a race-free
"exactly one session" guarantee. Do not just check-then-insert without
locking.

### D. `src/live/mod.rs`

Add:
```rust
pub mod guard;
pub mod holidays;
```

### E. `src/commands/state.rs`

Add fields to `AppState`:
```rust
pub live_guard: Arc<LiveGuard>,
pub pending_live_token: Arc<Mutex<Option<PendingLiveToken>>>,
pub ack_path: PathBuf,
pub app_handle: tauri::AppHandle,   // for event emission
pub event_bus: Arc<EventBus>,       // may already exist
```

`main.rs` wiring is Prompt 5's job — don't add to `main.rs` yet.

---

## Unit tests

In `src/live/guard.rs`, `#[cfg(test)] mod tests`:

- `test_token_expires`: build a `PendingLiveToken` with
  `expires_at = Instant::now() - Duration::from_millis(1)`. Wrapped in
  `Some(...)`, `validate_token` returns `Err`.
- `test_token_wrong_id`: token issued for `"strat-1"`; validate with
  `"strat-2"` → `Err`.
- `test_token_wrong_token`: correct id, wrong token string → `Err`.
- `test_token_valid`: fresh 90s token, correct fields → `Ok(())`.
- `test_token_none`: `validate_token(&None, ...)` → `Err`.

In `src/live/guard.rs`, market hours tests using the pure predicate:

- `test_market_open_9_15_monday`: 09:15 IST Monday, non-holiday → true.
- `test_market_open_9_14_monday`: 09:14 IST Monday → false.
- `test_market_open_15_30_monday`: 15:30 IST Monday → true.
- `test_market_open_15_31_monday`: 15:31 IST Monday → false.
- `test_market_closed_saturday`: 10:00 IST Saturday → false.
- `test_market_closed_sunday`: 10:00 IST Sunday → false.
- `test_market_closed_holiday`: pass a holiday date (from a test-only
  `NseHolidayCalendar` with a single known-holiday date) at 10:00 IST
  → false.

In `src/live/guard.rs`, ack file test:

- `test_read_ack_missing_file`: point at a non-existent path → false.
- `test_read_ack_true`: write `{"acknowledged": true}` to a temp file
  → true.
- `test_read_ack_false`: write `{"acknowledged": false}` → false.
- `test_read_ack_malformed`: write `not json` → false (safe default).

---

## After coding

Ask the user to run:

```
cargo test --lib live::guard
cargo test --lib
```

Wait for the output. If tests fail, ask for the full error text before
attempting a fix.

Do not proceed to Prompt 5 until:

- All prior tests still pass.
- All new `guard.rs` tests pass.
- The user confirms.

---

## Ask the user (at the end of this prompt)

The NSE holiday list changes yearly. Before finalising `holidays.rs`,
ask the user:

> "Please provide the current NSE trading holidays for {current_year} and
> {current_year + 1}. These change annually. Paste the list as YYYY-MM-DD
> lines, or point me to the NSE circular URL."

Do not fabricate a holiday list.
