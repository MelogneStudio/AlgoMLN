# AlgoMLN

**A fast, local-first algorithmic trading platform built in Rust.**

Built engine-first, not UI-first. Every layer is tested, deterministic, and production-grade before the next layer is added.

---

## Philosophy

Most trading platforms are built UI-first. AlgoMLN is built engine-first.

```
Data → Indicators → Strategy Engine → Backtesting → Execution → UI
```

- Fast cold start
- No cloud dependency
- No mandatory account
- Paper trading always default
- Deterministic backtests — same input, same output, every time
- One execution engine for backtests, paper trading, and live trading

---

## Current Status

```
Phase 1 (Data) · Phase 2 (Indicators) · Phase 2.5–2.9 (Strategy Engine) ✅
Phase 3–5 UI (Builder / Strategies / Coder / Uploader / Settings) ✅
Phase 6 (Plugin System — Rhai + WASM runtimes, capability gating) ✅
Phase 7 (Live Trading) — single live-session manager, trade log, 9-gate preflight ✅
Phase 8 (Engine eval/execute split + Plugin order gateway) — Tasks #1–#4 ✅, Tasks #5–#6 deferred
```

Run `cargo test --workspace` for the current test count
Current count: `341 passing | 1 ignored | 0 failed`.

### ✅ Phase 1 — Data Layer
- Broker abstraction trait (`BrokerClient`)
- `DhanClient` implementation
- Data models: `Candle`, `Tick`, `Quote`, `Order`, `Position`
- WebSocket manager — up to 1,000 symbol subscriptions, auto-reconnect
- Tick fan-out to internal subscribers
- Historical OHLCV fetch
- Tauri IPC commands exposing data to React
- Per-symbol live 1-minute candle assembly from ticks

### ✅ Phase 2 — Indicator Engine
Pure Rust functions. Stateless. `fn indicator(candles: &[Candle], period: usize) -> Vec<f64>`.

| Indicator | Function |
|---|---|
| Simple Moving Average | `ma` |
| Exponential Moving Average | `ema` |
| Relative Strength Index | `rsi` |
| Average True Range | `atr` |
| Volume Weighted Average Price | `vwap` |
| Relative Volume | `rel_vol` |
| Bollinger Bands | `bollinger_bands` → upper / mid / lower |

### ✅ Phase 2.5–2.9 — Strategy Engine

A complete strategy pipeline from source text to trade execution:

```
Source (.algomln)
  → Lexer
  → Parser
  → AST
  → Validator
  → Runtime Engine
  → ExecutionTarget (PaperBroker / LiveBroker)
```

**What's implemented:**

- Custom DSL with full compiler pipeline
- Trigger state system (fires only on `false → true` transitions)
- Cross detection (`cross_above`, `cross_below`)
- Indicator provider with bounded window (O(N) backtest performance)
- `PaperBroker` — cash, positions, avg entry price, realized PnL
- `ExecutionTarget` trait — same engine drives paper and live brokers
- Deterministic candle-by-candle backtest replay
- `behavioral_backtest` binary — run any `.algomln` file from the CLI

**Backtest performance on 184,863 candles (full NIFTY 1-min history):**
```
runtime: 3.5s · 52,000 candles/sec · 9,026 trades
```
> NOTE: This was tested not on my main setup but instead on an old i5 8th gen for normal person performance test. My main PC gets way more candles/sec.
---

## The Strategy Language

Strategies are written in `.algomln` files. The language is intentionally small — rules only, no variables, no loops.

### Grammar

```
strategy       = rule+
rule           = "WHEN" condition NEWLINE action

condition      = comparison
               | cross_expr
               | not_expr
               | logical_expr
               | position_expr    (parses, not yet evaluated)
               | time_window      (parses, not yet evaluated)

comparison     = expr operator expr
operator       = "<" | ">" | "<=" | ">=" | "==" | "!="

logical_expr   = condition "AND" condition
               | condition "OR" condition

not_expr       = "NOT" "(" condition ")"

cross_expr     = "cross_above" "(" expr "," expr ")"
               | "cross_below" "(" expr "," expr ")"

expr           = indicator_call | price_field | number

indicator_call = indicator "(" integer ")"
indicator      = "ema" | "ma" | "rsi" | "rel_vol" | "atr" | "vwap"
               | "bb_upper" | "bb_lower" | "bb_mid"

price_field    = "close" | "open" | "high" | "low" | "volume"
               | "prev_close" | "prev_open" | "prev_high" | "prev_low"

action         = "BUY" integer
               | "SELL" integer
               | "SELL" "ALL"
```

Blank lines and `# comments` are allowed anywhere. Keywords are case-insensitive. Indicator periods and quantities must be positive integers.

### Examples

**RSI oversold/overbought:**
```algomln
WHEN rsi(14) < 30
BUY 1

WHEN rsi(14) > 70
SELL ALL
```

**EMA crossover:**
```algomln
WHEN cross_above(ema(20), ema(50))
BUY 10

WHEN cross_below(ema(20), ema(50))
SELL ALL
```

**Compound condition:**
```algomln
WHEN ema(9) > ema(21) AND rsi(14) < 60
BUY 5

WHEN rsi(14) > 75
SELL ALL
```

**Bollinger Band breakout:**
```algomln
WHEN close < bb_lower(20)
BUY 10

WHEN close > bb_upper(20)
SELL ALL
```

---

## Plugin System

AlgoMLN can be extended without touching the engine. Plugins load from `<app_data>/plugins/<id>/` (each with a `plugin.toml` manifest) and run in one of two sandboxed runtimes:

- **Rhai** — a hardened script engine (op/recursion/collection budgets, no module loading, `print` swallowed).
- **WASM** — `wasmtime`-based, with a bounded linear memory limiter, epoch-interruption watchdog, and no WASI (plugins only ever see the `algomln::*` host functions).

Plugins only reach the engine through capability-gated accessors — a plugin must declare a capability (Market Data, Storage, Indicators, Analytics, DSL Extension, UI Panels, Scheduler, Execution) in its manifest, or the call is rejected with a permission error. Logging is the one always-on capability.

What plugins can currently do:

| Capability | What it gives the plugin |
|---|---|
| Indicators | Register a custom indicator function callable from `.algomln` strategies |
| Analytics | Register a custom backtest metric |
| DSL Extension | Register a new DSL keyword the parser/evaluator can resolve |
| Storage | A sandboxed per-plugin file-backed key/value store |
| UI Panels | Register a panel, push notifications/toasts, stream data into the panel |
| Scheduler | Cron-based recurring tasks |
| Market Data | Read-only access to the same broker client the strategy engine uses |
| Execution | Phase 7: read-only `positions()` snapshot (`ReadOnlyLiveExecutionApi`). Phase 8: `GatedLiveExecutionApi` — `submit_order` re-runs every engine gate (session, symbol, market hours, stale, cancelled, paused) and forwards through `DhanBroker::execute_with_meta`. |

Plugins publish and subscribe to engine events (`RuleFired`, `TradeExecuted`, `CandleProcessed`) over a broadcast event bus. **Backtests never wire up the event bus**, so plugin callbacks can't run during replay — this keeps backtests deterministic. The bus is only attached to the engine for paper/live runs.

Manage plugins from the desktop app's **Plugins** screen (list, enable, disable, reload) or via the `list_plugins` / `enable_plugin` / `disable_plugin` / `reload_plugins` Tauri commands.

---

## Running a Strategy

```powershell
# Run against full NIFTY 1-min history
cargo run --release --bin behavioral_backtest -- run my_strategy.algomln --data sample-data/nifty_1min.csv --symbol NIFTY

# Limit to first 10,000 candles
cargo run --release --bin behavioral_backtest -- run my_strategy.algomln --data sample-data/nifty_1min.csv --candles 10000

# Custom starting cash
cargo run --release --bin behavioral_backtest -- run my_strategy.algomln --data sample-data/nifty_1min.csv --cash 500000

# Run a named built-in profile
cargo run --release --bin behavioral_backtest -- profile rsi 50000
cargo run --release --bin behavioral_backtest -- profile ema

# Help
cargo run --release --bin behavioral_backtest -- --help
```

## Running the Desktop App

```powershell
# Install JS deps (first time only)
npm install

# Run Vite + Tauri together (hot-reload)
npm run tauri dev

# Frontend-only dev server (Rust not required; browser fallback for backtests/strategies)
npm run dev

# Type-check + production frontend build
npm run build
```

The Tauri app requires `DHAN_ACCESS_TOKEN` *and* `DHAN_CLIENT_ID` in `.env` (see `.env.example`). Live data, live order placement, and the funds-limit cache all need the access token; the client id is only required for `POST /orders`. The CLI loads `.env` automatically via `dotenvy`. Backtests work without a token — the CLI falls back to the bundled sample CSV and emits a stderr warning.

The live runner is **single-session** by design (Phase 7). The desktop app's **Live** screen shows status, open positions, pause/resume/stop controls, and the immutable trade log; the **Plugins** screen lists, enables, disables, and reloads loaded plugins. The browser fallback (`npm run dev`) renders the backtest flow but throws on every live command — you must use the Tauri window for live trading.

---

## Architecture

### Broker Abstraction

```
Strategy Engine
      ↓
ExecutionTarget trait
      ↓
┌─────────────┬─────────────┬──────────────┐
│ PaperBroker │  DhanBroker │ UpstoxBroker │
└─────────────┴─────────────┴──────────────┘
```

The engine never knows which broker is executing. `DhanClient` is implemented now. `UpstoxClient` slots in without touching anything else.

### Trigger State System

Without protection, `WHEN close > 0 / BUY 1` fires on every single candle. AlgoMLN uses a trigger state map that fires only on `false → true` transitions:

| Previous | Current | Fires? |
|---|---|---|
| false | true | ✅ yes |
| true | true | ❌ no |
| true | false | ❌ no |
| false | false | ❌ no |

### Deterministic Execution

Same strategy + same candles = identical results every run. No randomness, no non-deterministic data structures in evaluation paths. `BTreeMap` over `HashMap` wherever iteration order could affect output. This property is verified by a dedicated determinism test in the test suite.

### Cross Detection

Crossovers require tracking previous candle values per rule. `CrossDetector` stores `(fast_prev, slow_prev)` and fires only on the exact transition candle. The update pass runs after all rules are evaluated — not inside the rule loop — so all rules see consistent previous-candle state within a single cycle.

---

## Module Structure

The Rust crate lives at `src/` (not `src-tauri/src/` — that path is a Tauri shim that re-exports the lib).

```
src/
  broker/
    mod.rs            BrokerClient trait
    dhan/
      mod.rs          DhanClient — broker wiring
      auth.rs         DHAN auth/token handling
      rest.rs         REST client (historical OHLCV, etc.)
      websocket.rs    Live tick websocket
      models.rs       Dhan-specific request/response shapes
  models/             Candle, Tick, Quote, Order, Position
  indicators/         Pure indicator functions (one fn per file: ma, ema, rsi, atr, vwap, bb, rel_vol)
  feed/               WebSocket manager — up to 1,000 symbol subscriptions, auto-reconnect, tick fan-out
  strategy/
    dsl/
      lexer.rs        Lexer + Token types
      parser.rs       Recursive descent parser
      ast.rs          All AST node types (serializable)
      validator.rs    AstValidator — collects all errors
    runtime/
      engine.rs                StrategyEngine — main evaluation loop
      context.rs               EvalContext — per-candle borrowed view
      cross.rs                 CrossDetector
      trigger_state.rs         TriggerStateMap
      indicator_provider.rs    IndicatorProvider trait + BoundedWindowProvider
      incremental_provider.rs  IncrementalIndicatorProvider — streaming variant
    execution/
      target.rs        ExecutionTarget trait
      paper.rs         PaperBroker
      dhan.rs          DhanBroker (live execution + realized-loss / available-cash cache)
      order_builder.rs Builds Order from ActionNode
    logging/
      log.rs           StrategyLog, LogEntry, LogEntryKind
    analytics.rs      Backtest result metrics
    tests/            Integration tests (backtest_integration.rs)
  live/                Live session manager + safety gate layer
    session.rs         LiveSession — single-session tick loop, engine, lifecycle
    guard.rs          LiveGuard — 9-gate preflight + PendingLiveToken
    trade_log.rs      Append-only JSONL trade log
    candle_assembler.rs CandleAssembler — tick → 1-minute candle
    holidays.rs       NSE holiday calendar
  plugin/
    api/              Capability trait defs + per-capability impls (market data, storage,
                      indicator/analytics/DSL-extension registries, event bus, scheduler,
                      log, UI broadcast, no-op execution stub)
    runtime/
      rhai_runtime.rs Rhai script plugin — hardened engine, capability-gated host fns
      wasm_runtime.rs WASM plugin — wasmtime 48, bounded memory, epoch interruption
    host.rs           PluginHost — capability-gated `*_guarded` accessors
    loader.rs         Manifest → boxed Plugin (dispatches on entry file extension)
    registry.rs       In-memory plugin map, lifecycle (Loaded/Enabled/Disabled/Failed)
    manifest.rs       PluginManifest + PluginPermissions
    types.rs          PluginId, PluginMeta, Capability, PluginError
  commands/
    mod.rs            Re-exports
    data.rs           Tauri IPC commands — data / broker
    strategy.rs       Tauri IPC commands — backtest, deploy, list, validate
    registry.rs       StrategyRegistry — JSON-persisted deploy/list/status
    state.rs          AppState — struct held by Tauri::manage
    plugins.rs        list/enable/disable/reload plugin command bodies
    live.rs           request_live_start / confirm_live_start / acknowledge_live_trading / pause_live_strategy / resume_live_strategy / stop_live_strategy / get_live_status / get_trade_log
    indices.rs        list_indices / get_index_symbols / refresh_indices
  bin/
    behavioral_backtest.rs  CLI backtest runner (calls commands::strategy::run_backtest_internal)
src-tauri/
  src/main.rs         Tauri app entrypoint — registers commands, loads .env, sets up DataState,
                      wires plugin HostFactory, opens the trade log, starts index + symbol-map refreshes
```

### Frontend Layout

The React app lives at `src/` (the project root's `src/` — separate from the Rust `src/`). It's a thin client over Tauri IPC; there is no separate "live code path."

```
src/                       React frontend root (TypeScript, Vite, React 19)
  App.tsx                  Top-level orchestrator — screen/modal state, builder state, scale,
                           live_session_failed / live_session_stopped_with_positions toasts
  main.tsx                 Mounts <App /> into #root, loads global CSS tokens/fonts
  components/
    AppWindow/             Root shell; injects --ui-scale CSS variable
    TitleBar/              Custom title bar (data-tauri-drag-region)
    Sidebar/               Builder / Strategies / Live / Plugins / Settings nav; force-collapsed below scale 0.75
    Button/                Button primitive (primary | ghost | code variants)
    RuleRow/               One row of the visual strategy builder
    IndicatorPicker/       IndicatorKind dropdown
    NumberInput/           Numeric input control
    OptionSlider/          Reusable slider
    ScaleSlider/           Settings slider for --ui-scale
    LiveConfirmModal/      Three-step preflight → ack → confirm modal for `request_live_start`
    Toast/                 ToastContext — failure / open-positions notifications
  screens/
    Builder/               Main visual strategy builder + BacktestPanel
    Strategies/            List of deployed strategies (Tauri IPC: list_strategies)
    StrategyCoder/         Modal .algomln source editor
    StrategyUploader/      Modal .algomln file loader
    Settings/              UI scale, default capital, about; index refresh trigger
    Plugins/               List/enable/disable/reload loaded plugins
    Live/                  LiveScreen — status, open positions, pause/resume/stop, trade log table
  hooks/
    useStrategyBuilder     Builder state + loadFromDsl() round-trip
    useDslSync             Derives live DSL from builder state, debounced validate
    useBacktest            Runs runBacktest IPC; browser fallback synthesizes empty result
    usePlugins             list/enable/disable/reload plugin IPC + "plugin-ui-message" listener
    useLiveStatus          Polls get_live_status every 5 s — only place that polls the live IPC
  lib/scaling.ts           DESIGN_WIDTH/HEIGHT (1550x757), computeFitScale(), applyScale()
  types/                   tauri.ts (IPC wrappers + isTauri()), strategy.ts, backtest.ts, live.ts
```

---

## Roadmap

### Phase 3 — Charts & Core UI ✅ (basic shell)
- Visual strategy builder, strategies list, settings, coder modal, uploader modal
- Scaling shell (1550×757 logical canvas, --ui-scale CSS variable)
- Browser-only fallback (`npm run dev`) so the UI is demoable without Tauri

### Phase 4 — Trading Tools (pending)
- Option chain viewer
- Open Interest analysis
- Payoff diagrams
- Screener

### Phase 5 — Visual Strategy Builder ✅
```
Drag-and-drop blocks → Generated DSL → AST → Strategy Engine
```
Same runtime as text strategies. No separate execution path.

### Phase 6 — Plugin System ✅
```
Plugin manifest → PluginLoader → Rhai / WASM runtime → capability-gated PluginHost
```
- Rhai script runtime (hardened engine: op/recursion/collection budgets)
- WASM runtime (wasmtime 48, bounded memory, epoch-interruption watchdog, no WASI)
- Capability gating: Market Data, Storage, Indicators, Analytics, DSL Extension, UI Panels, Scheduler, Execution (stub)
- Broadcast event bus (`RuleFired` / `TradeExecuted` / `CandleProcessed`) — wired for paper/live only, never for backtests
- Desktop Plugins screen: list / enable / disable / reload

### ✅ Phase 6.5 — Advanced Strategy Features (partial)
- **Stop-loss / take-profit** — strategy-level `STOP_LOSS` / `TAKE_PROFIT` declarations on `StrategyNode` (not `RuleNode`s); bypass `TriggerStateMap` deliberately and run *after* the rule loop. Stop-loss wins on a gap candle.
- **Risk controls** — `RISK MAX_ORDERS`, `RISK MAX_POSITIONS`, `RISK MAX_DAILY_LOSS` strategy-level declarations; `check_risk_breach` runs before every order in `submit_action`. `MAX_DAILY_LOSS` is a hard non-negotiable for live trading (gate 7 of `LiveGuard`). `MAX_ORDERS` counts entry orders (BUYs) only — exits (`SELL` / `SELL ALL`, including the strategy-level SL/TP synthetic closes) are exempt so the safety net can always close a position (audit A2).
- **Position sizing** — `OrderBuilder::resolve_quantity` supports `QuantitySpec::Fixed` and `QuantitySpec::PercentCapital` (the latter is fed by `DhanBroker::available_cash`).
- **Multi-symbol strategies** — Phase 7 single-symbol only; the DSL `TRADE_IN Symbols` / `TRADE_IN NIFTY_*` clauses exist but the live runner rejects multi-symbol with a clear error.

**Live-execution audit resolutions (committed):**

| Finding | Resolution |
|---|---|
| **A1** `MAX_DAILY_LOSS` was a silent no-op in live trading | `check_risk_breach` now reads starting capital from `StrategyInstance::initial_cash` (threaded through `LiveSession::start` with `DEFAULT_LIVE_INITIAL_CASH = 1_000_000.0` INR). A stderr warning fires when `<= 0.0` so the user notices the cap is inactive. |
| **A2** `MAX_ORDERS` could block stop-loss / take-profit exits | Exits (`SELL`, `SELL ALL`, strategy-level SL/TP synthetic closes) short-circuit `check_risk_breach` before `session_orders` is touched. `session_orders` increments only on successful entry (BUY) executes. |
| **B1** no per-candle market-hours gate in the live tick loop | The tick loop runs `is_market_open(candle_close_ist, &holiday_calendar)` per candle. A session started at 15:29 will not submit an order after 15:30. Off-hours candles still land in `candle_history` so indicator windows stay continuous across weekend gaps. |
| **B2** live tick-loop gap-fill + flush | `CandleAssembler::feed` emits a zero-volume gap candle for every minute between the closed minute and the new tick. `CandleAssembler::flush()` returns the in-progress partial candle and is called from `cancel.cancelled()` and `RecvError::Closed`, so a stop at 15:29:30 preserves the last minute's prices and volume. Both gap-filled and flushed candles flow through the same B1-gated `append_candle` helper. |

### ✅ Phase 7 — Live Trading

- `DhanBroker` wraps `DhanClient` and implements `ExecutionTarget` — same engine drives backtests, paper, and live
- Single active `LiveSession` slot in `AppState` (tokio `Mutex<Option<Arc<LiveSession>>>`) — tick subscription via shared `FeedManager`, 1-minute candles via `CandleAssembler`, `StrategyEngine::on_candle` reused
- `LiveGuard::run_preflight` — 9-gate safety layer (paper-default, broker reach, symbol map, segment, market hours, risk controls, `MAX_DAILY_LOSS`, broker freshness, ack file) with a 90 s single-use `PendingLiveToken`
- Three-step UX: `request_live_start` → `acknowledge_live_trading` (one-time consent) → `confirm_live_start`
- `GET /funds/limit` cached every 60 s for `PercentCapital` order sizing (`DEFAULT_AVAILABLE_CASH_CAP` = 1 lakh INR on a cold cache); `get_positions`-backed realized-loss cache; **3 consecutive failures** mark stale → session auto-pauses
- `resume_live_strategy` is refused while the broker cache is stale (audit H2) — error cites the elapsed time since the last successful refresh
- Immutable append-only trade log (`<app_data>/trade_log.jsonl`) — only **fill** results are written; `Transit`/`Pending` orders are returned with status intact but no phantom row
- `pause_live_strategy` / `resume_live_strategy` / `stop_live_strategy` / `get_live_status` — session lifecycle IPC; `stop` cancels the tick task under a 5 s drain timeout and queries open positions *after* stop
- Loud failure signalling — `SessionEventEmitter` trait + `live-session-failed` Tauri event
- Plugin `Execution` capability is **read-only** in Phase 7 (`ReadOnlyLiveExecutionApi`) — order submission is Phase 8
- Browser fallback: every live command throws `"live trading is not available in the browser"`; the modal swaps Step 1 for a "live trading is only available in the desktop app" notice

Full audit of the live-execution path lives in `plans/live_execution/live_execution_audit.md` (the C1/C3/H1/H2/H4/M2/L4 resolution table is there, plus the new top-down `A1`/`A2`/`B1`/`B2` fixes in `plans/total_audit/total-audit.txt`). C2 (`OrderIntents` refactor) and H3 (cancel in-flight HTTP) are fixed in Phase 8 — see below.

### ✅ Phase 8 — Eval/Execute Split + Plugin Order Gateway (partial)

- **`StrategyEngine::plan_candle` / `execute_intent`** — the engine now splits evaluation from execution. `plan_candle(&[Candle]) -> (Vec<LogEntry>, Vec<OrderIntent>)` returns the audit trail of intents (order, rule id, notes — `"stop_loss"` / `"take_profit"` / empty, candle timestamp) without firing the broker. `on_candle` stays as a thin compatibility wrapper for backtests and the CLI; the live tick loop plans under the engine lock, drops the lock, and executes each intent through a cancellation-aware executor. Backwards-compatibility guarded by `on_candle_preserves_logs_when_run_via_plan_candle`.
- **`GatedLiveExecutionApi`** — the plugin order gateway. Plugins that declare the `Execution` capability can now submit live orders. Every `submit_order` re-runs six gates before forwarding to `DhanBroker::execute_with_meta`: session active, symbol resolves to `Segment::NseEq`, market hours via `is_market_open`, broker not stale, session `CancellationToken` not cancelled, and BUY blocked when paused (SELL always passes so plugin-driven exits remain allowed). A `PluginSessionContextGuard` RAII handle stages `strategy_id = "plugin:<strategy_id>"` and `strategy_name = "<strategy_name> [plugin order]"` on the broker so the trade-log row attributes to the plugin. `cancel_order` returns a structured error (broker has no cancel endpoint yet); `positions()` mirrors the read-only path. The Tauri host factory wires `GatedLiveExecutionApi` when the session slot is `Some` and `NoopExecutionApi` otherwise.
- **H3 cancellation plumbing** — `LiveSession::start` constructs a `CancellationToken` and wires it into the broker via `DhanBroker::set_cancel_token`. `execute_with_meta` checks `cancel_token.is_cancelled()` at the very top, before the paused-for-entries check or the broker HTTP call — `stop()` mid-`place_order` aborts within a few hundred ms and the trade-log row is never appended. Token lives in a `parking_lot::RwLock<CancellationToken>` so the broker can be shared through `Arc<DhanBroker>` without exclusive ownership.
- **C2 lock discipline** — `execute_intent` still keeps the lock today because extracting `risk_state` and the logger from `&mut self` is a much larger refactor; H3 cancellation is the practical safety net. The full lock-release refactor (release during the await on `execute`) is deferred to Phase 9 multi-session work.
- **Remaining** — Tasks #5–#6 (W1/W2/W5 wiring, L2/L8 hardening) deferred to a follow-up commit.



---

## Tech Stack

```
Tauri (Rust + React)
  Rust backend   — all logic, data, indicators, strategy engine
  Tauri commands — direct IPC, no localhost server
  React frontend — charts and UI via webview
```

---

## Long-Term Vision

Research → Strategy Design → Backtest → Paper Trade → Live Trade

All inside one application. One execution engine. No hidden paths. No discrepancies between backtest and live results.
