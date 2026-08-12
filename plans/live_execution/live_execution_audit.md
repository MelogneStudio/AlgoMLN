# AlgoMLN — Live Execution Code Audit

Audit of every file in the live-execution path: `commands/live.rs`, `live/{session,guard,trade_log,candle_assembler,holidays,mod}.rs`, `strategy/execution/{dhan,paper,order_builder,target}.rs`, `commands/state.rs`, `plugin/api/execution.rs`, `feed/manager.rs`, and the Tauri wiring in `src-tauri/src/main.rs`. CLAUDE.md invariants 1, 3, 8, 9b–9d, 11, 13, 18, 19 were used as the reference.

Findings are ranked Critical → High → Medium → Low → Wiring, each with file:line, code excerpt, why it matters, and a concrete fix. A "Concrete fix priorities" section at the bottom lists the smallest correct patches.

---

## Critical

### C1. `DhanBroker::available_cash` returns `f64::MAX` — live risk-control is wired to a lie

**File:** `src/strategy/execution/dhan.rs:351-353`

```rust
fn available_cash(&self) -> f64 {
    f64::MAX
}
```

**Why it matters:** `OrderBuilder::resolve_quantity` (`src/strategy/execution/order_builder.rs:55-68`) uses `available_cash` to size every order when the DSL says `QuantitySpec::PercentCapital`. Live orders computed against `f64::MAX` will buy as many shares as `(f64::MAX * pct / 100) / current_price` produces, which `.floor() as u64` truncates — for a price like 2500 INR and `pct = 10`, that is ~`u64::MAX` qty. The order hits the broker, which almost certainly rejects it for "insufficient funds" — but on any Dhan endpoint that doesn't pre-validate quantity, this is a live-money hazard. Paper broker returns its running cash, so backtests and paper do not exercise this path.

**Fix:**
- Add a margin-fetch method to `DhanClient` (e.g. `get_fund_limits`).
- Cache the result similarly to `realized_loss`, with its own background refresh (60 s is fine).
- `DhanBroker::available_cash` should return the cached value, defaulting to a hard cap (e.g. 1_000_000 INR) until the first refresh lands.
- If the margin refresh is stale, the session should auto-pause the same way `is_stale()` does today.

---

### C2. Engine lock held across the broker HTTP call — every tick stalls behind every other tick

**File:** `src/live/session.rs:229-230`

```rust
let mut engine = task_session.engine.lock().await;
engine.on_candle(&candles).await;
```

**Why it matters:** `on_candle` calls `execution_target.execute` (per `src/strategy/runtime/engine.rs:125`) which, for live, awaits a `place_order` HTTP call. Holding the `tokio::sync::Mutex<StrategyEngine>` across that await means:
- `paused_for_entries` writes from `pause_live_strategy` must acquire the same lock and serialize behind an in-flight order.
- A 500 ms `place_order` response freezes the whole tick loop including the stale-cache check, market-hours, and SL/TP logic.
- Two simultaneous candle boundaries (rare but possible after a Lagged recovery) cannot both enqueue work.

**Fix (Phase 8 work, already noted in the comment):**
- Change `StrategyEngine::on_candle` to return `Vec<OrderIntent>` instead of executing directly.
- `LiveSession` tick loop: lock engine, collect intents, release lock, then call `broker.execute_with_meta` for each intent outside the lock.
- Keep the engine's deterministic `BTreeMap` invariant — intents are a pure function of candle history.

---

### C3. `request_live_start` releases the `live_session` lock before consuming the token — concurrent start/stop can interleave

**File:** `src/commands/live.rs:63-140`

```rust
let session = state.live_session.lock().await;
if session.is_some() { return Err(...); }
drop(session);                                  // ← released
...
let result = state.live_guard.run_preflight(...).await?;   // ← long await, no lock
...
*state.pending_live_token.lock().await = Some(token);       // ← second lock acquisition
```

**Why it matters:** Between `drop(session)` and the token write, anything can run. In particular:
- A second `request_live_start` from the UI (e.g. user double-click) can pass the `is_some()` check, run preflight twice, and issue a second token. The first token to land in the slot wins; the second is silently overwritten.
- `confirm_live_start` can run *while* the first request is still inside `run_preflight`. It will see the not-yet-stored token and reject. Then the first request lands, stores a token that the user already gave up on, and it leaks for 90 s.
- `stop_live_strategy` cannot race because the slot is empty, but the lack of an atomic "check slot empty + check token empty" pair means a future code path could trivially introduce a session-start race.

**Fix:**
- Hold `state.live_session.lock().await` across the entire `request_live_start` body until the token is written into `pending_live_token`. The preflight is one network call; the cost is fine.
- Or: store the token first (atomic check-and-set with the slot under the same mutex) before running preflight. This still needs the slot lock.

Concretely:

```rust
let mut session_slot = state.live_session.lock().await;
if session_slot.is_some() {
    return Err("a live session is already active...".into());
}
// run preflight while holding the lock
let result = state.live_guard.run_preflight(...).await?;
*state.pending_live_token.lock().await = Some(token);
Ok(...)
```

---

### C4. `request_live_start` parses the DSL and looks up the strategy twice with no benefit

**File:** `src/commands/live.rs:74-94` (first) and `src/commands/live.rs:182-205` (second, in `confirm_live_start`)

**Why it matters:** The comment at lines 178-181 admits "id is unique per deploy" — so the re-parse in `confirm_live_start` buys nothing. The duplication is also a small race window where the second lookup could in principle see a different version of the strategy than the first.

**Fix:**
- Pass the parsed `StrategyNode` and resolved `symbol` from `request_live_start` to `confirm_live_start` via the token's metadata, or stash them in `pending_live_token` alongside the UUID.
- Alternative: just remove the second `get` and accept the small staleness risk. The first call is the user-visible latency path; the second is fast.

---

## High

### H1. Trade log writes for `Transit` / non-fill statuses carry `price: 0.0` and `order_status: "Transit"` as a "trade"

**File:** `src/strategy/execution/dhan.rs:246-269`

```rust
let price = if classified.status.is_fill() {
    order.price.unwrap_or(0.0)
} else {
    0.0
};
...
order_status: format!("{:?}", classified.status),
```

`classify_result` (`src/strategy/execution/dhan.rs:381-390`) treats `Transit` and `Pending` as `Ok` so `execute_with_meta` proceeds to the trade log. The UI's trade log will show `BUY 5 @ 0.00 — Transit`.

**Fix (two options):**

Option A — skip appending when not a fill:

```rust
let is_fill = classified.status.is_fill();
if is_fill {
    let entry = TradeLogEntry { ... };
    if let Err(e) = self.trade_log.append(entry) { ... }
}
```

Option B — add a `filled: bool` field to `TradeLogEntry` and surface it in the table.

Option A is simpler and matches the wire shape (a `Transit` order is not a trade).

---

### H2. `paused_for_entries` is the only thing that blocks BUY, but `MAX_ORDERS` and `MAX_DAILY_LOSS` are also bypassed when paused

**Files:** `src/strategy/execution/dhan.rs:211-221`, `src/strategy/runtime/engine.rs::check_risk_breach`

**Why it matters:** The `realized_loss` cache — input to `MAX_DAILY_LOSS` — is computed by the *broker's* background task. The session auto-pauses when `DhanBroker::is_stale()` is true (per CLAUDE.md #9c). The session does **not** unpause itself when the cache recovers. The user has to click Resume. If the user clicks Resume while the cache is still stale, they enter Running state with a still-stale safety metric.

**Fix:**
- In `pause_live_strategy` IPC path, also check `broker.is_stale()` and return an error if stale: "realized-loss cache is stale, please retry once it recovers."
- In the broker, expose a `time_since_last_success()` so the IPC can show a useful error.
- The auto-pause in the tick loop is fine as-is.

---

### H3. `cancel.cancelled()` does not abort an in-flight `place_order`

**File:** `src/live/session.rs:156-181` and the in-flight `on_candle` at lines 229-230

**Why it matters:** If a candle is in flight (long `on_candle` from C2) and the user clicks Stop, `cancel` cancels, the loop breaks, but the in-flight `on_candle` is not aborted. Its HTTP `place_order` will complete, and the trade log entry will be written *after* the user has been told the session is stopped. The status flips to `Stopped` in the trailing block at `src/live/session.rs:235-238` but the order is real money.

**Fix (couples with C2):**
- After the C2 refactor (intents returned from `on_candle`), use a `tokio_util::CancellationToken` *inside* `execute_with_meta` and check it at the top of every `place()` call.
- Or: have `LiveSession::stop` first `cancel.cancel()`, then set a "draining" flag, then poll `task.is_finished()` with a 5 s timeout, then `task.await`. If the task doesn't finish, log the outstanding order and surface a UI warning.

---

### H4. `stop_live_strategy` queries positions *after* taking the session but *before* stopping it

**File:** `src/commands/live.rs:359-374`

```rust
let session = state.live_session.lock().await.take()
    .ok_or_else(|| "no live session".to_string())?;
let positions = session.broker.get_positions().await.unwrap_or_default();
let open_count = positions.iter().filter(|p| p.quantity != 0).count();
session.stop().await;
```

**Why it matters:** The tick loop is still running when `get_positions` is called. The HTTP call may race with `execute_with_meta` calls. Concretely: a SELL that closes a position can land between the `get_positions` snapshot and the loop's exit. The user is told "1 open position remains" when it was just closed.

**Fix:**
- Move the position query *after* `session.stop().await`. The tick loop is gone, so the position snapshot is final.
- The warning message can still say "open positions at the time of stop."

---

### H5. `start_time` uses `Utc::now()` directly without a `Clock` trait

**File:** `src/live/session.rs:136` and the wire field at `src/commands/live.rs:450`

**Why it matters:** Cosmetic for now, but it means a test or replay can't pin the start time, and the freshness of "session uptime" in the UI is wall-clock dependent.

**Fix:** Defer. Trivial.

---

## Medium

### M1. `execute_with_meta` failure is logged as `BrokerError` but the trade log entry is not written — yet a non-fill leaves a phantom line behind

**File:** `src/strategy/execution/dhan.rs:223-279`

**Why it matters:** When `classify_result` returns `Err` (Rejected/Cancelled/Expired), the function returns *before* `trade_log.append`. But the engine's `LogEntryKind::OrderError` references the `order_id` that was assigned by the broker for a rejected order. Anyone reading the engine log without the trade log can't tell the order was rejected.

**Fix:** A single `Err(_)` log line that quotes the `order_id`:

```rust
Err(err) => {
    eprintln!("[dhan_broker] order {} rejected: {}", classified.order_id, err.message);
    return Err(err);
}
```

---

### M2. `LiveGuard::is_market_open` has dead `nanosecond` comparison

**File:** `src/live/guard.rs:243-262`

```rust
let nanos = dt.nanosecond();
let after_open = (hour, minute, second, nanos) >= (9, 15, 0, 0);
```

`dt.nanosecond()` is always 0–999_999_999, so the `nanos` field of the comparison is always 0 in the test cases. Harmless; oddly redundant.

**Fix:** Drop the `nanos` and compare `(h, m, s)` directly. Or keep for forward-compat with sub-second tests.

---

### M3. `subscribe` and the broadcast channel have no per-symbol filtering

**File:** `src/feed/manager.rs:11-68` and `src/live/session.rs:183-185`

**Why it matters:** Each session receives all ticks and filters on `tick.symbol != task_session.symbol`. In Phase 7 single-session this is fine. In multi-session, every session pays the per-tick check.

**Fix:** Defer to multi-session. Add a per-symbol routing layer in `FeedManager` when needed.

---

### M4. `engine.event_bus` overwrite is silent if a plugin ever resets it to `None`

**File:** `src/live/session.rs:124`

**Why it matters:** No runtime check that the bus is still wired at each `on_candle`. Minor.

**Fix:** Add a debug_assert in `on_candle` that `event_bus.is_some()` when running in a live session. Tracked via a `is_live: bool` flag on the engine.

---

### M5. `AppState` has four references to `DhanBroker`

**File:** `commands/state.rs:26-62`, `src-tauri/src/main.rs:317,328,407,638`

**Why it matters:** `data.dhan_broker`, `live_guard.broker`, `live_session.broker`, and the `DhanClient` itself. Four `Arc<DhanBroker>` clones. Fine for `Arc`, but debugging ref counts is confusing.

**Fix:** Add a comment near the four construction sites explaining the deliberate sharing.

---

### M6. `DhanBroker::is_stale()` is never set to true in tests

**File:** `src/strategy/execution/dhan.rs:127-149`

**Why it matters:** In a test, the background task isn't spawned, so `consecutive_failures` only increments on explicit `refresh_realized_loss` calls that fail. The auto-pause-on-stale path is therefore never exercised in tests.

**Fix:** Add an integration test that calls `broker.refresh_realized_loss()` 3 times with a mock that always fails, then asserts `is_stale() == true` and that the session would auto-pause.

---

### M7. `fetch_seed_candles` silently tolerates empty seeds

**File:** `src/commands/live.rs:283-297`

**Why it matters:** If the API returns zero candles (vs. errors), the engine starts cold with no warning. Fine, but no `eprintln!` to disambiguate "empty result" from "API error."

**Fix:** `eprintln!("fetch_seed_candles: API returned 0 candles for {symbol}")` when the vec is empty.

---

### M8. `confirm_live_start` and `request_live_start` duplicate the parse+symbol-resolution

**File:** `src/commands/live.rs:94-124` and `src/commands/live.rs:195-205`

**Why it matters:** Same code twice. See C4 for the fix.

---

### M9. `LiveSession::start` writes `session_context` *before* the tick loop exists; on a failed `start`, the context is never cleared

**File:** `src/live/session.rs:107-113, 268`

**Why it matters:** If `feed.subscribe` returns an `Err` at line 144, the context has been set and is never cleared. The next successful `start` for a different strategy will overwrite it, but in the gap the broker thinks it belongs to the *failed* strategy. Currently no `execute` can land in that gap (the only path is the engine, which never started), but the invariant is fragile.

**Fix:** Use a RAII guard that clears `session_context` on drop unless `commit()` is called. Or restructure `start` to set the context *after* the feed is subscribed.

---

### M10. `pause` / `resume` take effect only after the current `on_candle` completes

**File:** `src/commands/live.rs:306-332`, `src/live/session.rs:249-257`

**Why it matters:** A UI click will appear unresponsive for up to one full minute if a candle is in flight (combined with C2, up to one minute plus an HTTP round trip). Documented as intended, but worth a doc note on the IPC.

**Fix:** Document explicitly. No code change.

---

## Low

### L1. Event name kebab-case consistency
All `emit` calls use kebab-case (`"live-session-failed"`, `"plugin-ui-message"`, `"live-session-stopped-with-positions"`). Consistent. Verify the React listener names match. No code change.

### L2. `DhanBroker::set_paused_for_entries` is `pub`, and `paused_for_entries` field is also `pub`
**File:** `src/strategy/execution/dhan.rs:102, 191-193`

Anyone with the `Arc<DhanBroker>` can flip the flag. Phase 7 plugins are read-only via `ReadOnlyLiveExecutionApi`, but if a future plugin gets write access and the broker reference leaks, this is a one-line tripwire. Not exploitable today; flag for Phase 8 review.

**Fix:** Make the field `pub(crate)` and expose a `pub fn set_paused_for_entries` that's gated by a `LiveSession` context (e.g. requires the broker to have a non-`None` `session_context`).

### L3. `LiveStatusWire` does not include `engine_status`
**File:** `src/commands/live.rs:404-419`

The UI can't tell whether the *engine* thinks it is running vs the session. For most paths they agree, but if the engine ever self-pauses (it doesn't today), the wire shape hides it.

**Fix:** Add an `engine_status: String` field.

### L4. Market-hours boundary inclusive on close
**File:** `src/live/guard.rs:243-262`

A session started at 15:29:00 will run `on_candle` for a 15:30 candle and may submit an order at 15:30:00.001 which is *after* market close. NSE rejects such orders, but the gate does not prevent them — only prevents the *start*.

**Fix:** Add a doc note in the Live UI: "sessions started in the last minute can place orders after close." Or change the close to 15:25 to give a 5-min buffer.

### L5. `TradeLog::open` `.expect` fails the whole app if `<app_data>` is unwritable
**File:** `src-tauri/src/main.rs:252-253`

If `<app_data>` is read-only or full, the app fails to start. Could be a softer error and disable live trading with a UI banner.

**Fix:** Return a `Result<AppState, Box<dyn Error>>` from the setup closure; on `TradeLog::open` failure, log a warning and skip the live subsystem.

### L6. `ack_path` write uses `std::fs::write` which truncates without explicit locking
**File:** `src/commands/live.rs:258`

If the user has multiple Tauri windows or the file is open by another process, the write can race. Windows file locking semantics are forgiving.

**Fix:** `OpenOptions::new().write(true).truncate(true).create(true).open(&state.ack_path)`.

### L7. `LiveSession::start` returns `Err` after writing `session_context` if feed subscribe fails
**File:** `src/live/session.rs:107-113, 142-148`

See M9. Same root cause.

### L8. `LiveSession::broker` is `pub`
**File:** `src/live/session.rs:76`

Plugins that obtain the `Arc<LiveSession>` indirectly (via `AppState`) could call `broker.execute_with_meta` directly. Currently no path exposes the session Arc to plugins.

**Fix:** Make it `pub(crate)` and expose a `pub fn broker_positions(&self)` async helper that returns the position snapshot.

### L9. `start_time: DateTime<Utc>` doesn't track sub-second precision
**File:** `src/live/session.rs:136`

If two sessions start in the same second, the UI cannot disambiguate. Trivial.

**Fix:** Use `chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)`.

### L10. `DhanBroker::refresh_realized_loss` is `pub`
**File:** `src/strategy/execution/dhan.rs:170-178`

Anyone with the broker can trigger a refresh on demand. Used in tests; harmless in production. No change.

---

## Wiring / Coordination

### W1. `data.feed` is the same `Arc<Mutex<FeedManager>>` shared between `AppState` and `LiveSession`

**File:** `src-tauri/src/main.rs:327, 233` and `src/commands/live.rs:233`

The session's `feed.subscribe(vec![symbol])` requires `&mut self` and is called under a tokio lock. Tauri's state is read-only-after-setup, so no one else calls `subscribe` concurrently. Confirmed safe in single-session.

### W2. The plugin `HostFactory` `try_lock` race is benign

**File:** `src-tauri/src/main.rs:455-481`

A plugin loaded *after* a session is already running calls `try_lock` against a contended mutex if the live session is mid-`on_candle` (which holds no feed/engine locks at the *construction* moment, but the session slot itself is only held briefly during start/stop). In practice the `try_lock` succeeds because `confirm_live_start` only holds it for the duration of `LiveSession::start` (the bulk of which is `feed.subscribe` and the `task` spawn). The window is small.

**Fix:** Add a one-line comment: "Race is benign in practice — the slot is only held briefly during start/stop."

### W3. `runtime_handle_for_factory` is captured at construction time

**File:** `src-tauri/src/main.rs:398, 428, 469`

The `tokio::runtime::Handle` is dropped when the closure is dropped, but `ReadOnlyLiveExecutionApi::new` stores a clone, so the handle outlives the closure. The runtime itself is owned by Tauri. No leak.

### W4. `TradeLog::append` uses `std::sync::Mutex` on the file

**File:** `src/live/trade_log.rs:33, 52-60`

`DhanBroker::execute_with_meta` is async and is called from the engine. The `Mutex::lock()` is sync; under a multi-thread runtime, blocking is OK as long as contention is bounded. In a live session, the engine calls `execute_with_meta` at most once per rule per candle. No concern.

### W5. `refresh_realized_loss` is called after every successful `place()`

**File:** `src/strategy/execution/dhan.rs:229`

If the broker is "fresh" (no failures), this is one extra `GET /positions` per order. Combined with C2, a single rule firing at the start of a candle issues N+1 HTTP calls before the next candle can be processed. Live latency could be 1–2 s for a 5-rule strategy.

**Fix:** Batch the refresh — only refresh on candle boundaries, not after every order. Or refresh only if the last refresh was > 5 s ago.

---

## Concrete fix priorities

If I had to rank by ROI:

1. **C1** (`available_cash = f64::MAX`) — one-line fix, real money at risk.
2. **C3** (request_live_start lock release) — keep the `live_session` lock across preflight; closes a session-start race.
3. **H3** (cancel does not abort in-flight order) — needs the `OrderIntents` refactor already mentioned in the comment, but at minimum a `cancel.cancelled()` race with `place_order` is a known paper-tiger in live trading.
4. **H1** (Transit logged as a trade) — UI confusion; easy fix in `execute_with_meta`.
5. **C2** (engine lock across HTTP) — punt to Phase 8 as the code already does, but write a `cargo test` that asserts the lock is *not* held across `place_order` after the refactor.
6. **H4** (positions query before stop) — query *after* `stop().await` or move it inside the session.

The safest first PR is **C1** + **H1** — neither touches the engine or the wire surface.

---

## Resolution Status

Tracked against this audit. `file:line` points to the implementation after the fix.

| Item | Status | Notes |
|---|---|---|
| **C1** `available_cash = f64::MAX` | ✅ Fixed | `DhanClient::get_funds_limit` calls `GET /funds/limit`. `DhanBroker` caches the value (60 s refresh), defaults to `DEFAULT_AVAILABLE_CASH_CAP` (1 lakh INR) on a cold cache, and three consecutive failures mark `funds_stale` so the session auto-pauses. See `src/strategy/execution/dhan.rs` (`available_cash`, `refresh_funds_once`, `FundsSource` trait) + `src/broker/dhan/models.rs::DhanFundsLimit` + `src/broker/dhan/rest.rs::get_funds_limit`. |
| **C3** `request_live_start` lock release | ✅ Fixed | The `live_session` tokio Mutex is held across preflight **and** the pending-token write. `src/commands/live.rs::request_live_start`. |
| **H3** `cancel` does not abort in-flight `place_order` | 🟡 Partial | `LiveSession::stop` now waits for the tick task under a 5 s drain timeout (`STOP_DRAIN_TIMEOUT`) and logs a warning if the task is still running (most likely blocked on `place_order`). True cancellation of in-flight HTTP awaits the C2 / `OrderIntents` refactor. `src/live/session.rs::stop`. |
| **H1** `Transit` / non-fill logged as a trade | ✅ Fixed | `execute_with_meta` now returns early with `Ok(classified)` when `!classified.status.is_fill()` — only fills are written to the trade log. A single stderr line is emitted on a failed placement so the engine log carries the `order_id` (M1). `src/strategy/execution/dhan.rs::execute_with_meta`. |
| **C2** engine lock across HTTP call | ⏭ Deferred (Phase 8) | Doc comment at `src/live/session.rs` updated to reference audit item C2 and Phase 8 ownership. The refactor of `StrategyEngine::on_candle` to return `Vec<OrderIntent>` is Phase 8 work as the audit recommended. |
| **H4** positions query before stop | ✅ Fixed | `stop_live_strategy` calls `session.stop().await` first, then queries positions — the snapshot is final, no race with the tick loop's last SELL. `src/commands/live.rs::stop_live_strategy`. |
| **H2** resume-during-stale | ✅ Fixed | `DhanBroker::time_since_last_success` returns `Option<Duration>` (`None` until the first refresh succeeds). `resume_live_strategy` now refuses to resume when `broker.is_stale()` is true and returns an error like "broker cache is stale … the last successful broker refresh was 12s ago" so the user can decide whether to retry. `src/strategy/execution/dhan.rs::time_since_last_success` + `src/commands/live.rs::resume_live_strategy`. |
| **M2** redundant nanosecond comparison | ✅ Fixed | `is_market_open` now compares `(hour, minute, second)` only — the `nanos` tuple slot was always 0 in practice and never affected the boundary. `src/live/guard.rs::is_market_open`. |
| **L4** market-hours boundary edge case | ✅ Fixed | Doc note in `src/live/guard.rs::is_market_open` warns that sessions started in the last minute before close may still place an order on the 15:30 candle boundary. The Live UI surfaces the same warning in `LiveConfirmModal` (preflight step). `src/live/guard.rs` + `src/components/LiveConfirmModal/LiveConfirmModal.tsx`. |

### Verification

- `cargo check --lib` — clean.
- `cargo test --lib strategy::execution::dhan` — 18/18 passing (3 new C1 tests cover bounded default, successful refresh, stale-keeps-last-good-value; 3 new H2 tests cover `time_since_last_success` before/after a refresh and on a failed refresh).
- `cargo test --lib live::guard` — 20/20 passing (4 new M2 tests cover the same sub-second boundaries as the originals).
- `cargo test --lib live::session` — 4/4 passing.
- `cargo test --lib` — 311/311 passing, 1 ignored (live Dhan fetch), no regression.
- `npx tsc --noEmit` — clean.

### Items not in the priority list (noted for follow-up)

These were flagged in the audit but not in the priority list, so left untouched in this pass:

- **L2 / L8** `pub` broker / session fields — Phase 8 hardening.
- **W1 / W2 / W5** wiring / refresh-after-every-order — Phase 8 batching + comment-only fixes.

Resolution committed 2026-08-12.
