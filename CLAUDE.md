# CLAUDE.md

AlgoMLN is a local-first algorithmic trading platform — engine-first Rust, thin Tauri + React UI. Strategies compile from a custom DSL to an AST and are evaluated candle-by-candle deterministically against historical (or live) data, then routed to an `ExecutionTarget` (Paper or Live broker). Backtests, paper trading, and live trading share one engine — there is no separate "live code path."

```
Data → Indicators → Strategy Engine → Backtesting → Execution → UI
```

> **Documentation map.** This file is the index — project overview, run commands, invariants, and pointers. Narrative and "where to find what" details live in:
> - **`ARCHITECTURE.md`** — file tree + "where to look for X" lookup tables (DSL → which files, indicators → which files, IPC → which files, etc.)
> - **`BACKEND.md`** — narrative on the Rust crate: DSL pipeline, runtime/evaluation loop, execution targets, Tauri commands, CLI, data flow
> - **`FRONTEND.md`** — narrative on the React app: shell/scaling, screen state machine, builder↔coder round-trip, IPC hooks, wire types

## Common Commands

Run from the repo root. Rust is the source of truth; the UI is a thin client.

### Tests
```powershell
cargo test --workspace                  # full Rust test suite
cargo test --lib strategy::             # only strategy tests
cargo test --lib -- test_name_substring # single test by name substring
```

### Backtest CLI
`behavioral_backtest` runs any `.algomln` strategy against CSV data. Sample NIFTY 1-min data lives in `sample-data/`.

```powershell
cargo run --release --bin behavioral_backtest -- run my_strategy.algomln --data sample-data/nifty_1min.csv --symbol NIFTY
cargo run --release --bin behavioral_backtest -- run my_strategy.algomln --data sample-data/nifty_1min.csv --candles 10000 --cash 500000
cargo run --release --bin behavioral_backtest -- profile rsi 50000
```

### Tauri App
```powershell
npm install            # first time
npm run tauri dev      # Vite + Tauri together
npm run dev            # frontend-only dev server
npm run build          # type-check + production frontend build
```

There is no separate linter step — `npm run build` runs `tsc` and acts as the de-facto type check.

### First-time setup

```powershell
python scripts/fetch_seed_indices.py   # populates src-tauri/resources/indices/*.json (stdlib only)
```

Run this once from the repo root before the first `npm run tauri dev` so the bundled index seed files are present. The Rust app also tolerates a missing seed (each alias falls back to an empty list and the user can refresh in Settings).

## Critical Architecture Invariants

Don't break these — they are non-obvious properties the codebase is built around.

**1. Determinism.** Same strategy + same candles = identical output. Use `BTreeMap` over `HashMap` anywhere iteration order could affect evaluation. No randomness on eval paths. The engine uses `BTreeMap`-backed registries. (Guarded by a dedicated test.)

**2. Trigger state machine.** A bare `WHEN x > y / BUY 1` would fire every candle. `TriggerStateMap` fires rules only on `false → true` transitions. `CrossDetector` stores `(fast_prev, slow_prev)` and fires on the exact transition candle. Critically: the cross update pass runs **after** all rules are evaluated for the cycle — not inside the rule loop — so every rule sees consistent previous-candle state within a single cycle. See `src/strategy/runtime/cross.rs` + `trigger_state.rs`, called from `engine.rs::on_candle`.

**3. One execution engine.** Backtests, paper, and live all run through `StrategyEngine` + `ExecutionTarget`. The engine never knows which broker is behind the target. `PaperBroker` and `DhanBroker` are interchangeable implementations. (`src/strategy/execution/`.)

**4. Indicator windows are bounded.** `BoundedWindowProvider` keeps indicators O(N) — 184,863 NIFTY 1-min candles complete in ~3.5s (~52k candles/sec). Don't refactor into naive full-history recomputation. (`src/strategy/runtime/incremental_provider.rs`.)

**5. DSL grammar is intentionally small.** Rules only — no variables, no loops. Keywords case-insensitive. Indicator periods and quantities must be positive integers. `position_expr` and `time_window` parse but are not yet evaluated at runtime — see `src/strategy/dsl/ast.rs` and the `NotYetImplemented` branch in the evaluator.

**6. Wire-format boundary.** Rust types crossing the IPC boundary serialize to camelCase JSON (`#[serde(rename_all = "camelCase")]`). Internal `BacktestResult` / `PaperTrade` keep snake_case Rust field names; the conversion happens in `commands::strategy::BacktestResultWire` and `PaperTradeWire`. If you add a field the UI consumes, add it to the wire struct AND mirror it on the TS side — don't put extra UI-relevant fields directly on `BacktestResult` and rely on Rust serde to rename.

**7. Plugin capability gating.** Plugins reach the engine only through `PluginHost`'s `*_guarded` accessors (`src/plugin/host.rs`). The accessor checks the plugin's declared `Capability` list and returns `PluginError::PermissionDenied` if the capability is missing. `LogApi` is intentionally unguarded. The plugin layer ships capability implementations for market data (broker-backed), per-plugin file KV storage, indicator/analytics/DSL-extension registries with plugin-id dedup, a no-op execution stub (until the engine is wired to a broker-agnostic facade), a broadcast event bus, cron scheduling, and a Tauri-broadcast UI API — see `src/plugin/api/`. The Rhai script runtime is live in `src/plugin/runtime/rhai_runtime.rs` (hardened engine, capability-gated host fns, lifecycle dispatch). The WASM plugin runtime is live in `src/plugin/runtime/wasm_runtime.rs` (wasmtime 23, capability-gated `algomln::*` host fns, bounded linear memory via `ResourceLimiter`, epoch-interruption-armed store; WASI is intentionally not linked because `WasiCtx` in wasmtime 23 is `Send`-only and would violate the `Plugin: Send + Sync` bound). The plugin loader (`src/plugin/loader.rs`) dispatches on entry extension to `RhaiPlugin` or `WasmPlugin`. The plugin registry (`src/plugin/registry.rs`) holds the in-memory plugin map, calls each plugin's `on_load` with a `PluginHost` built by a caller-supplied factory, and tracks `Loaded` / `Enabled` / `Disabled` / `Failed` per plugin. The plugin API surface in `src/plugin/api/mod.rs` is plugin-id-attributed: `IndicatorRegistryApi::register` / `AnalyticsApi::register_metric` / `DslExtensionApi::register_keyword` all take a `PluginId`; `SchedulerApi::cancel_all_for(plugin_id)` and `DslExtensionApi::unregister_all_for(plugin_id)` let the registry reclaim per-plugin entries on disable.

**7a. Plugin log rate-limiting and rolling files.** The Tauri host factory wires every plugin's `LogApi` to a `RateLimitedFileLog` (`src/plugin/api/log_file.rs`) so a misbehaving or malicious plugin cannot fill the user's SSD by spamming `log_info` / `log_warn` / `log_error`. The rate limiter is a per-plugin token bucket: default 10 msg/sec burst, 100 msg/min sustained, shared across all log levels; excess messages are silently dropped, with a single per-minute `[plugin:<id>] [WARN] rate-limited: N message(s) dropped in the last 60s` summary line so the user can see throttling is happening. Output goes to `<app_data>/logs/plugin-<id>.log` and is capped at 5MB per file — on every write that would push the on-disk size past 5MB the current file is renamed to `<base>.1` (older `*.1` overwritten) and a fresh current file is opened. The CLI binary keeps using the simpler `NamespacedLog` because it does not load plugins; the CLI path is for backtesting and profiling only.

**8. Engine event-bus gating.** `StrategyEngine` carries an `event_bus: Option<Arc<EventBus>>` (default `None`) and publishes three event kinds from `on_candle` only when the bus is `Some`: `RuleFired` after `TriggerStateMap::should_fire` returns `true` (rule-eval pass), `TradeExecuted` immediately after a successful `execution_target.execute` (paper broker recovered via `as_any` downcast on `ExecutionTarget`), and `CandleProcessed` after the cross-update pass. No event is published from the cross-update or indicator-advance passes themselves. **Backtests deliberately leave `event_bus` as `None`** (see `commands::strategy::run_backtest_internal`) so plugin callbacks never run during replay — determinism is preserved. The Tauri binary owns the live bus and assigns it to the engine only for paper/live runs (stage 9 hook in `src-tauri/src/main.rs`).

**9. Tauri plugin wiring.** `AppState` is defined in `src/commands/state.rs` (re-exported as `commands::AppState`) and carried by `tauri::Manager` so `State<'_, AppState>` works in every command. The Tauri binary's `setup` closure builds the plugin's shared infrastructure once — registries, event bus, scheduler, broker wrappers, noop execution — and wires them into a single `HostFactory` closure. `PluginRegistry::scan_and_load` runs at startup via `tauri::async_runtime::block_on` (Tauri 2's `setup` is sync). `TauriUiApi` messages are forwarded to the Tauri event bus as `"plugin-ui-message"` via a `tokio::spawn`'d forwarder; the React app subscribes once and dispatches on the `UiMessage` variant. `#[tauri::command]` wrappers for plugin commands live in `src-tauri/src/main.rs` (not the library) because the macro generates module-private artifacts that `tauri::generate_handler!` must resolve in the same scope. The four plugin commands are `list_plugins` / `enable_plugin` / `disable_plugin` / `reload_plugins`. `PluginRegistry::enable` / `disable` / `unload` swap the real plugin out of the entry under the write lock before awaiting the lifecycle callback (parking_lot guards are `!Send`, so holding one across `.await` would break Tauri's command dispatcher and risk deadlock if a plugin re-enters the registry).

**9a. Tauri index + symbol-map startup wiring.** After `scan_and_load`, the setup closure also loads `IndexRegistry` from `<app_data>/indices/` (falls back to bundled `src-tauri/resources/indices/`) and `SymbolMap` from `<app_data>/sec_id_cache.csv` (falls back to the bundled seed in `sample-data/sec_id.csv` or the bundled resource `src-tauri/resources/sample-data/sec_id.csv`). Two background tasks check both for staleness (90 days for indices, 7 days for the symbol map) and refresh silently on startup. The three index IPC commands — `list_indices` / `get_index_symbols` / `refresh_indices` — live in `src/commands/indices.rs` and are registered alongside the plugin commands in `tauri::generate_handler!`. `refresh_indices` is long-running (~30–60s) and writes both the index JSON and the symbol-map CSV before swapping the in-memory `SymbolMap` under its `RwLock`.

**10. Index data is read-only at runtime.** `IndexRegistry::update` is the only mutator; it is called only by `refresh_index` (and `load_from_dirs` at startup). Strategy engines read constituents once at deploy time and do not re-read mid-run. If constituents change (quarterly rebalance), re-deploy the strategy.

**11. PortfolioEngine sub-engines are never accessed concurrently.** `PortfolioEngine::on_tick` takes `&mut self` and is called from a single tokio task; the underlying `StrategyEngine` instances share a single `Arc<PaperBroker>`. `PaperBroker` is internally `Mutex`-guarded so the shared broker is `Send`, but the per-sub-engine state (cross detector, trigger map, indicator window) is `!Sync`. Adding parallel `on_tick` calls would require `Mutex<StrategyEngine>` per sub-engine and a careful determinism audit — do not add it without a full review.

**12. SL/TP is a safety net, not a rule.** `STOP_LOSS` and `TAKE_PROFIT` are strategy-level declarations on `StrategyNode` (not `RuleNode`s) that bypass `TriggerStateMap` deliberately. They fire every candle the position is underwater or in profit, not just on a `false → true` transition. The pass runs *after* the rule loop and the cross-state update so a rule that closes the position on the same candle is reflected. Stop-loss wins on a gap candle that would trigger both — the position is already closed by the time take-profit would check. (`src/strategy/runtime/engine.rs::run_stop_loss_take_profit_pass`, `src/strategy/dsl/parser.rs` for declarations, `src/strategy/dsl/validator.rs` for the `(0, 100]` range check.)

**13. Risk controls are evaluated before every order.** `RISK MAX_ORDERS`, `RISK MAX_POSITIONS`, and `RISK MAX_DAILY_LOSS` are strategy-level declarations (like SL/TP) on `StrategyNode.risk: Option<RiskConfig>`. The engine stores the per-run counter in `StrategyEngine::risk_state: Option<RiskState>` (allocated only when at least one declaration is present). `check_risk_breach` runs at the top of `submit_action` — before the order is built — and logs `LogEntryKind::RiskBreach { rule_id, reason }` on a hit, then returns without submitting. `session_orders` is incremented **only on a successful `execute`** so a failed order does not consume the cap. `MAX_POSITIONS` only blocks BUYs; sells (including SL/TP `SELL ALL`s) are never blocked by it. `MAX_DAILY_LOSS` is session-scoped (cumulative realized loss as a percentage of `initial_cash`); in a backtest there is no real clock, so "daily" spans the whole run. (`src/strategy/runtime/engine.rs::check_risk_breach`, `src/strategy/dsl/parser.rs::parse_risk_declaration`.)

## Test Layout

- Per-module unit tests live alongside source as `#[cfg(test)] mod tests` blocks.
- Strategy integration tests live in `src/strategy/tests/` (notably `backtest_integration.rs`).
- A dedicated determinism test guards the same-input-same-output property.
- No frontend test suite is wired up — verify UI changes manually via `npm run tauri dev`.

## Env Requirement

Tauri app requires `DHAN_ACCESS_TOKEN` in `.env` (see `.env.example`). The CLI loads `.env` via `dotenvy::dotenv()`. When the token is missing or Dhan returns no candles, `run_backtest_dsl` falls back to the bundled sample CSV and emits an `eprintln!` warning.

## Instructions for editing code

1. On any edit made, at the end of the message, along with telling the user all the changes, also update all the md files (`ARCHITECTURE.md`, `BACKEND.md`, `FRONTEND.md`, `CLAUDE.md` and if it is a big change, `README.md`. Only update with data in your context unless specified to fully check and update readme or if you only have to read a few files.
2. Prefer storing changes in your mind and grouping write / edit commands
