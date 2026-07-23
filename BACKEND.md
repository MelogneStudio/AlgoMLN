# BACKEND.md

Narrative on the Rust crate — what's in it, how it fits together, and why it's shaped that way. For file-by-file lookup, see `ARCHITECTURE.md`. For invariants, see `CLAUDE.md`.

---

## What the backend does

The backend is a single Rust crate (`algomln` library + `behavioral_backtest` binary) that takes a user's `.algomln` strategy text, compiles it to an AST, evaluates it candle-by-candle against historical (or live) market data, routes the resulting orders to a pluggable execution target, and emits structured results back to either the CLI or the Tauri webview.

There is exactly one evaluation loop (`StrategyEngine`) and exactly one execution abstraction (`ExecutionTarget`). Paper trading, backtesting, and live trading all run through the same engine; swapping the broker is a constructor argument, not a code path.

```
┌──────────────┐   .algomln     ┌──────────────┐
│ Source text  │ ─────────────► │   Lexer      │
└──────────────┘                └──────┬───────┘
                                       │ tokens
                                ┌──────▼───────┐
                                │   Parser     │
                                └──────┬───────┘
                                       │ StrategyNode (AST)
                                ┌──────▼───────┐
                                │  Validator   │  (period > 0, qty > 0, dup rule ids, …)
                                └──────┬───────┘
                                       │ validated AST
                                ┌──────▼───────┐  per candle
                                │   Engine     │ ◄──── EvalContext, BoundedWindowProvider,
                                │ on_candle()  │       CrossDetector, TriggerStateMap
                                └──────┬───────┘
                                       │ ActionNode (BUY/SELL)
                                ┌──────▼───────┐
                                │ order_builder│
                                └──────┬───────┘
                                       │ Order
                                ┌──────▼───────┐
                                │ ExecutionTarget (trait)  ──► PaperBroker | DhanBroker
                                └──────────────────┘
```

---

## The DSL pipeline

Source text goes through three stages. The whole pipeline is shared between the Tauri `validate_dsl` IPC and the backtest orchestrator.

**Lexer** (`src/strategy/dsl/lexer.rs`). Pure character-to-token conversion. Token kinds include keywords (`WHEN`, `BUY`, `SELL`, `AND`, `OR`, `NOT`, `CROSS_ABOVE`, …), indicator names, price fields, comparison operators, and number/integer literals. Errors carry `line` and `col` so the UI can highlight them.

**Parser** (`src/strategy/dsl/parser.rs`). Recursive-descent parser that consumes the token stream and produces a `StrategyNode { name, trade_in, stop_loss, take_profit, rules: Vec<RuleNode> }`. Each `RuleNode` has a unique `id` (assigned as `rule_{N}` during parsing) so log entries and trigger state can be keyed by rule. The optional `STOP_LOSS <pct>%` and `TAKE_PROFIT <pct>%` declarations can appear anywhere in the source (before, after, or between rules); duplicate declarations of the same field are a parse error.

The grammar is intentionally tiny — see `CLAUDE.md` "The `.algomln` DSL" for the full EBNF. `position_expr` and `time_window` parse into the AST but the runtime evaluates them as `NotYetImplemented`; the parser was extended ahead of the runtime.

**AST** (`src/strategy/dsl/ast.rs`). All enums and structs are `Serialize + Deserialize` so they round-trip cleanly through the IPC boundary if needed. `ConditionNode` is a flat enum (`Comparison`, `CrossAbove`, `CrossBelow`, `And`, `Or`, `Not`, `InPosition`, `TimeWindow`) — the parser only builds the first three and a couple more, but the AST is the source of truth for what the runtime understands.

**Validator** (`src/strategy/dsl/validator.rs`). Rejects empty strategies, zero quantities, non-positive indicator periods, duplicate rule IDs, invalid time ranges, crossovers that mix an indicator with a literal (since a literal can't change), and STOP_LOSS / TAKE_PROFIT values outside `(0.0, 100.0]`. Validation runs after parsing for both `validate_dsl` and the backtest orchestrator, so the engine can assume a well-formed AST.

`commands::strategy::validate_dsl` (in `src/commands/strategy.rs`) is the thin Tauri-facing wrapper that returns `Vec<String>` of human-readable errors with `"line {l} col {c}: {msg}"` formatting for lex/parse errors and plain messages for validation. The strategy registry has its own local copy of the same pipeline (`validate_dsl_local` in `src/commands/registry.rs`) to avoid creating a cyclic module dependency.

### Index registry (multi-symbol strategies)

`IndexRegistry` (`src/indices/registry.rs`) holds a `parking_lot::RwLock<HashMap<IndexAlias, IndexEntry>>` of NSE index constituent symbol lists. The 22 entries are populated once at startup from two sources, in order:

1. **User cache** — `<app_data>/indices/*.json`. Updated by `refresh_index` after a successful fetch.
2. **Bundled seed** — `src-tauri/resources/indices/*.json` (shipped in the app bundle via `tauri.conf.json`).

For any alias that fails to load from both locations, the registry inserts an empty entry (`symbols: vec![], last_updated: "never"`) so the key always exists and the strategy engine can distinguish "loaded zero" from "not loaded."

`refresh_all_if_stale` (`src/indices/refresh.rs`) is the background refresh. It checks the staleness of `nifty_50.json` as a proxy and, if older than `DEFAULT_STALENESS` (24h), iterates all 22 indices and fetches each CSV from `niftyindices.com/IndexConstituent/`. Each successful fetch is written to `<app_data>/indices/<stem>.json` and pushed into the registry via `IndexRegistry::update`. Failures are logged to stderr and the app keeps running with whatever is already loaded. The Tauri setup closure (`src-tauri/src/main.rs`) spawns `refresh_all_if_stale` once at startup.

Per invariant #10 in `CLAUDE.md`, the registry is read-only after startup from the strategy engine's perspective. Constituents can change on quarterly rebalances; the deploy-time read guarantees the same strategy never re-evaluates with a different universe mid-run.

### Symbol map (NSE → Dhan security ID)

`SymbolMap` (`src/broker/symbol_map.rs`) is a `HashMap<String, u32>` mapping uppercase NSE equity symbols to Dhan `SECURITY_ID`s. The map is loaded once at startup from the bundled `sample-data/sec_id.csv` (a snapshot of the Dhan scrip master). Parsing filters to `EXCH_ID=NSE, SEGMENT=E` rows only; `SYMBOL_NAME` wins over `UNDERLYING_SYMBOL`; first-occurrence-wins for duplicates.

The map is held in `AppState.symbol_map: Arc<parking_lot::RwLock<SymbolMap>>` so a future hot-refresh (`refresh_symbol_map`) can swap the map without restarting the app. The Dhan scrip master URL is `https://images.dhan.co/api-data/api-scrip-master-detailed.csv`; the refresh writes to `<app_data>/symbol_map.csv` via a temp file + atomic rename. Runtime wiring of `SymbolMap` into the engine is a follow-up prompt — for now the map is built and held in `AppState` so the IPC surface is in place.

---

## The runtime / evaluation loop

`StrategyEngine` lives in `src/strategy/runtime/engine.rs`. One method, `on_candle(&mut self, candles: &[Candle]) -> Vec<LogEntry>`, advances the engine by one candle. The CLI and Tauri commands call it in a `for index in 1..=candles.len()` loop.

The structure of `on_candle` is the single most important thing to understand in the codebase:

1. **Cache the rule list** (`self.instance.strategy.rules.clone()`) so the rest of the loop can run without holding a borrow on `self.instance.strategy`.
2. **First pass — evaluate every rule.** For each rule:
   - `eval_condition` returns `Result<bool, EvalError>`.
   - `TriggerStateMap::should_fire(rule_id, condition_result)` returns true only on a `false → true` transition. Bare `WHEN x > y` would otherwise fire every candle.
   - If fired, the engine logs the condition evaluation, the rule fire, the order submission, the execution result (or skip/failure).
3. **Second pass — update crossover state.** After *all* rules are evaluated for this cycle, walk the rules again and call `CrossDetector::update(rule_id, fast, slow)`. Doing this *after* the rule loop guarantees that within a single cycle, every rule sees the same `prev` state — there is no ordering hazard where the first rule's cross-detector update affects the second rule's evaluation. This is invariant #2 in `CLAUDE.md`.
4. **Stop-loss / take-profit pass** (only when either is declared on the strategy). Calls `execution_target.get_positions()` to snapshot the open position, computes unrealized loss/gain against the entry price from `Position::average_price`, and submits a synthetic `SELL ALL` through the normal order path if either threshold is breached. Stop-loss fires first; take-profit is skipped on a gap candle that would have triggered both because the position is already closed. See `run_stop_loss_take_profit_pass` in `engine.rs`.
5. **Advance the indicator window.** `BoundedWindowProvider::advance` pushes the current candle into the rolling 500-candle window and drops the oldest if the cap is hit.
6. **Drain the logger** and return the entries to the caller. The CLI and Tauri both append these to the final `BacktestResult.logs`.

### Stop loss / take profit

`STOP_LOSS <pct>%` and `TAKE_PROFIT <pct>%` are strategy-level declarations (not rules) that can appear anywhere in the source — before, after, or between rules. Both are optional and validated independently to be in `(0.0, 100.0]`. They are stored on `StrategyNode` as `Option<f64>` and parsed/validated by the existing DSL pipeline.

The SL/TP pass deliberately bypasses `TriggerStateMap` (it's a safety net, not an edge-triggered rule): every candle the position is open and underwater or in profit, the pass fires and submits a `SELL ALL` order. The two `LogEntryKind` variants `StopLossFired { symbol, loss_pct, price }` and `TakeProfitFired { symbol, gain_pct, price }` are logged before the order is submitted; the order itself logs through the standard `OrderSubmitted` / `OrderExecuted` path with `rule_id = "stop_loss"` or `"take_profit"`. If both thresholds would fire on the same candle, stop-loss wins and the take-profit pass is skipped.

The pass runs after the rule loop and the cross-state update so any rule-driven position change on the same candle is reflected in the SL/TP check. `Position::average_price` already carries the entry price — no new field was needed on the wire.

### Risk controls

`RISK MAX_ORDERS <int>`, `RISK MAX_POSITIONS <int>`, and `RISK MAX_DAILY_LOSS <pct>%` are strategy-level declarations (same pattern as `STOP_LOSS` / `TAKE_PROFIT`) parsed into `StrategyNode.risk: Option<RiskConfig>`. All three are optional; any combination is valid. The declarations can appear anywhere alongside rules and SL/TP, and each sub-keyword is a separate `RISK` line — duplicate declarations for the same field are a parse error.

- `MAX_DAILY_LOSS` must be in `(0.0, 100.0]`; `MAX_POSITIONS` and `MAX_ORDERS` must be `>= 1`. Validation lives in `validator.rs` alongside the SL/TP threshold check.
- The engine stores the per-run counter in `StrategyEngine::risk_state: Option<RiskState>` (allocated only when at least one declaration is present). `check_risk_breach` runs at the top of `submit_action` and logs `LogEntryKind::RiskBreach { rule_id, reason }` if any limit is breached; the order is not built or submitted.
- `MAX_ORDERS` counts orders that `execution_target.execute` accepted. Failed orders (insufficient cash / position) do not consume the cap.
- `MAX_POSITIONS` only blocks `BUY` actions — sells (including SL/TP `SELL ALL`s) are never blocked by it. The count comes from `execution_target.get_positions()` and includes only positions with `quantity > 0`.
- `MAX_DAILY_LOSS` is session-scoped: the engine sums the negative `PaperTrade.pnl` values via `ExecutionTarget::realized_loss()` and computes `loss_pct = realized / initial_cash * 100.0`. In a backtest there is no real clock, so "daily" spans the whole run. When breached, all subsequent orders (buys and sells) are blocked.

The engine is profiled (`StrategyEngineProfile`): it counts `on_candle` calls, broker `execute` calls, and broker `get_positions` calls, and accumulates elapsed time. The backtest orchestrator packages these into `EngineProfileReport` and `IndicatorProfileReport` and ships them to the UI for the "Throughput" panel in the CLI summary.

### Indicator provider

`IndicatorProvider` (trait in `src/strategy/runtime/indicator_provider.rs`) has two implementations:

- **`BoundedWindowProvider`** (`incremental_provider.rs`) — the production one. Maintains a rolling 500-candle window and a `HashMap<(IndicatorKind, usize), f64>` cache that is cleared at the start of every `on_candle` cycle. Indicators are computed on the rolling window (not the full history), so the work per candle is O(window) = O(max indicator period) instead of O(full history). On 184,863 NIFTY 1-min candles the engine completes in ~3.5s (~52k candles/sec). This is invariant #4.
- **`FullRecomputeProvider`** — the naive implementation. Kept around for the bench test in `indicator_provider.rs` so future refactors can compare against it.

The `latest_indicator_value` helper is the single dispatch point from `IndicatorKind` to a concrete function in `src/indicators/`. It also strips `NaN` / `Inf` via `is_finite()` so a partial-window indicator returns `None` cleanly.

### Crossover detection

`CrossDetector` (`src/strategy/runtime/cross.rs`) stores `(fast_prev, slow_prev)` per rule in a `BTreeMap` (deterministic iteration, invariant #1). It fires on the exact transition candle (`fast_prev <= slow_prev && fast_curr > slow_curr`) and stays silent thereafter until the next crossover. `is_cross_above` / `is_cross_below` are pure reads; `update` is the only mutator.

### Trigger state

`TriggerStateMap` (`src/strategy/runtime/trigger_state.rs`) is even simpler — a `BTreeMap<rule_id, bool>` that fires on a `false → true` transition. Both structures are independent per rule id.

### Portfolio engine (multi-symbol paper / live)

`PortfolioEngine` (`src/strategy/portfolio/engine.rs`) fans a single parsed `StrategyNode` out over a list of symbols and runs each symbol's rules in its own `StrategyEngine`, all sharing one `Arc<PaperBroker>`. Capital is shared across symbols; positions are tracked per symbol inside the broker's existing `HashMap<String, PaperPosition>`.

- Constructors:
  - `PortfolioEngine::new(&strategy, symbols, initial_cash, event_bus)` — explicit symbol list. The symbol list is uppercased and stored in insertion order for deterministic logging. Panics if `symbols` is empty.
  - `PortfolioEngine::from_trade_in(&strategy, &trade_in, &index_registry, initial_cash, event_bus)` — resolves a `TradeIn::Symbols` or `TradeIn::Index` clause via `resolve_trade_in_symbols` and then calls `new`. Returns `Err` for `TradeIn::Index` when the alias has no symbols loaded (the user must refresh from Settings).
- Dispatch: `on_tick(&mut self, symbol, candles)` finds the matching sub-engine (case-insensitive) and calls `on_candle`. Unknown symbols log a warning to stderr and return an empty log vec; the dispatch is single-threaded (invariant #11).
- Broker access: `broker()` returns the shared `Arc<PaperBroker>` for position/PnL snapshots. A live paper run reads `get_state().positions` to render the per-symbol holdings.
- `commands::strategy::resolve_trade_in_symbols` is a re-export of the same function so Prompt 3's Tauri commands can call it from the commands module without depending on `strategy::portfolio` directly.

Backtests with a `TRADE_IN` clause return an error from `commands::strategy::run_backtest_dsl` — multi-symbol backtest is not implemented. The deployment path (`PortfolioEngine::from_trade_in`) is the only currently-supported multi-symbol route.

---

## Execution

`ExecutionTarget` is the trait the engine talks to (`src/strategy/execution/target.rs`):

```rust
#[async_trait]
pub trait ExecutionTarget: Send + Sync {
    async fn execute(&self, order: Order) -> Result<OrderResult, ExecutionError>;
    async fn get_positions(&self) -> Result<Vec<Position>, ExecutionError>;
    /// Sum of negative realized `PaperTrade.pnl` values, returned as a
    /// positive number. Used by the engine's `RISK MAX_DAILY_LOSS` check.
    fn realized_loss(&self) -> f64;
    fn available_cash(&self) -> f64;
    fn is_paper(&self) -> bool;
    fn name(&self) -> &str;
}
```

The engine never imports a concrete broker type — it only knows the trait. Backtests construct an `Arc<PaperBroker>`, live trading will construct an `Arc<DhanBroker>`, and the same engine code drives both.

**`PaperBroker`** (`src/strategy/execution/paper.rs`). A `Mutex<PaperBrokerInner>` wrapping `cash: f64`, `initial_cash: f64`, `positions: HashMap<String, PaperPosition>`, and `trade_history: Vec<PaperTrade>`. Buys deduct cash and update a weighted average entry price; sells realize P&L against that average. Pushing `update_unrealized(symbol, current_price)` is the CLI's job once per candle (see `run_backtest_internal`) so the position's `unrealized_pnl` stays fresh.

**`order_builder`** (`src/strategy/execution/order_builder.rs`). Converts an `ActionNode` plus current price and current position into an `Order`. `SELL ALL` is resolved against the live position quantity — if there's no position it returns `OrderBuildError::NoPosition`, which the engine logs as a `RuleSkipped` entry rather than a hard error. The CLI test for `SELL ALL` with no position is in `order_builder.rs`.

---

## Backtest orchestration

`commands::strategy::run_backtest_internal` (`src/commands/strategy.rs`) is the central backtest routine. It:

1. Re-validates the AST.
2. Constructs a `PaperBroker` and a `StrategyInstance` (with `id = "backtest-strategy"`, `status = Running`).
3. Walks every candle in order, calling `engine.on_candle(&candles[..index]).await` and appending the returned log entries.
4. Pulls the broker state and the engine + indicator profiles.
5. Calls `BacktestAnalyser::analyse` to compute the `BacktestSummary` (win rate, drawdown, profit factor, etc. — see `src/strategy/analytics.rs`).
6. Returns a `BacktestResult` with the trade history, broker state, logs, summary, and profile reports.

The Tauri-facing variant `run_backtest_dsl` (in the same file) is the orchestrator the IPC calls. It runs the lex/parse/validate pipeline on the raw DSL source, then:

- Tries to fetch candles from the live broker via `data::DataState.broker.get_ohlcv(symbol, M1, now-7d, now)`.
- Falls back to the bundled `sample-data/nifty_1min.csv` if the broker is unreachable, returns no candles, or the symbol is unrecognizable. The fallback emits a stderr warning so the user knows.
- Refuses to run if both sources produce zero candles.

After the backtest, it converts the internal `BacktestResult` to `BacktestResultWire` (a strict subset matching the TS `BacktestResult` interface). The conversion turns `PaperTrade.timestamp: i64` into a string to match the TS side, but leaves the internal `i64` alone for analytics code. This is invariant #6.

---

## Tauri commands and the strategy registry

`src-tauri/src/main.rs` is a thin shell. Each `#[tauri::command]` is a one-liner that grabs `State<'_, AppState>` and forwards to a library function. `AppState` (defined in `src/commands/state.rs` and re-exported as `commands::AppState`) carries the `DataState` (broker + feed), an `Arc<StrategyRegistry>`, an `Arc<PluginRegistry>`, and a `tokio::sync::broadcast::Receiver<UiMessage>`.

Registered commands:

- `get_ohlcv`, `get_quote`, `subscribe_ticks` — live broker / feed (`commands::data`).
- `run_backtest` — `commands::strategy::run_backtest_dsl(...)`. Returns `BacktestResultWire`.
- `validate_dsl` — `commands::strategy::validate_dsl(source) -> Vec<String>`. Empty vec = valid.
- `deploy_strategy` — `registry.deploy(name, dsl_source, mode)`. Validates the DSL, generates `strat-{ms}-{counter}` id, persists a new record. New strategies default to `Paused`.
- `list_strategies` — `registry.list()`. Returns `DeployedStrategy` records sorted by `deployed_at` ascending.
- `set_strategy_status` — `registry.set_status(id, status)`. Flips the status and persists.

The **registry** (`src/commands/registry.rs`) is intentionally minimal: it is a JSON-persisted store of deploy/list/set_status operations and a *stub* for execution. It does not schedule ticks or run a live engine — the engine lifecycle is wired separately. The storage path is `app_data_dir/strategies.json` (Windows: `%APPDATA%\com.algomln.app\strategies.json`, identifier from `src-tauri/tauri.conf.json`).

`StrategyRegistry::open` reads the file (or creates an empty one) and builds an in-memory `HashMap<id, DeployedStrategyRecord>`. Deploys and status changes take the mutex, mutate, then write the full snapshot back to disk (small file, simple semantics). The on-disk record has `deployed_at` for sort order; the wire `DeployedStrategy` drops it and replaces the single `mode` with a `modes: [mode]` array to match the TS side.

`StrategyMode::parse` and `StrategyStatus::parse` accept case-insensitive inputs and reject anything outside the known set, so the UI can't pass typos through.

### Tauri commands — indices and symbol map

`src/commands/indices.rs` exposes three Tauri commands backed by the shared `IndexRegistry` and `SymbolMap` in `AppState`:

| Command | Args | Returns | Purpose |
|---|---|---|---|
| `list_indices` | — | `Vec<IndexInfo>` (alias, display name, count, last-updated) | Snapshot of all 22 indices for the Settings UI |
| `get_index_symbols` | `alias: String` | `Vec<String>` | Constituent list for a named alias (e.g. `"NIFTY_50"`) |
| `refresh_indices` | — | `RefreshResult` (refreshed, failed, symbol_map_updated, symbol_map_count) | Re-fetch all 22 indices from niftyindices.com and refresh the Dhan symbol map. Long-running (~30–60s); show a loading state in the UI. |

`refresh_indices` writes per-index JSON to `<app_data>/indices/<stem>.json` and the Dhan scrip master CSV to `<app_data>/sec_id_cache.csv` (atomic temp+rename). On a successful symbol-map refresh it swaps the in-memory `Arc<RwLock<SymbolMap>>` so subsequent IPC calls see the new map without restarting the app.

**Startup wiring** (in `src-tauri/src/main.rs::setup`): after the plugin shared infrastructure is built, the closure constructs `IndexRegistry`, calls `load_from_dirs(&cache_dir, &resource_dir)` (cache first, bundled seed second), and spawns a background task that calls `refresh_all_if_stale(...)` with a 90-day staleness window. The symbol map is loaded by trying `<app_data>/sec_id_cache.csv` first, then falling back to the bundled seed (`sample-data/sec_id.csv` in the repo root or `src-tauri/resources/sample-data/sec_id.csv` in the bundled resource dir). A second background task checks the cache with a 7-day staleness window and refreshes via `refresh_symbol_map(...)` when stale. Both background refreshes are non-fatal — failures are logged to stderr and the app keeps running with the seed data.

---

## The CLI

`src/bin/behavioral_backtest.rs` is a self-contained binary that avoids spawning Tauri. It has three subcommands plus a default test-suite run:

- **`run <file.algomln> --data <csv> [--candles N] [--cash N] [--symbol X]`** — load strategy + CSV, truncate candles, run `run_backtest_internal`, print a formatted summary.
- **`profile <name> [candles]`** — load the bundled NIFTY sample, run a built-in strategy (`rsi` or `ema`), print the throughput-focused summary. Used for benchmarking the engine.
- **`backtest <file.algomln> --security <id> [--from YYYY-MM-DD] [--to YYYY-MM-DD] [--exchange X] [--instrument Y] [--timeframe 1m|5m|…]`** — fetch from Dhan directly and run. Requires `DHAN_ACCESS_TOKEN` in `.env`.

The CLI also has a default mode (no subcommand) that runs three tiny deterministic tests against `sample-data/tiny_candles.csv` — useful for spot-checking the engine after a refactor.

`block_on` is a local tokio helper that builds a single-thread runtime on demand so the CLI doesn't need to be `#[tokio::main]`.

---

## Data

`src/data/csv.rs` is the shared CSV loader. `load_nifty_candles(path)` opens a file, skips the header, and parses each row with `parse_market_row`, which tries tab-separated (5 fields), comma-separated (5 fields), and whitespace-separated (6 fields, the first 19 chars are the timestamp). The loader returns `Result<Vec<Candle>, String>` so it's directly callable from Tauri commands without an `anyhow` round-trip.

The bundled `sample-data/nifty_1min.csv` is the offline fallback when Dhan is unreachable, has no token, or returns no candles for the requested symbol.

---

## Logging

`src/strategy/logging/log.rs` defines `LogEntry { id, timestamp, strategy_id, candle_timestamp, kind }` and a `LogEntryKind` enum covering: condition evaluated (with prev state and indicator snapshots), rule fired, order submitted, order executed, rule skipped (with `RuleSkipReason`), order failed, eval error, status changed, stop-loss fired (`StopLossFired { symbol, loss_pct, price }`), and take-profit fired (`TakeProfitFired { symbol, gain_pct, price }`).

`StrategyLogger` is per-engine (one per `StrategyInstance.id`). `on_candle` calls `drain_entries()` at the end of the cycle and the engine returns the drained vector; the backtest orchestrator appends it to `BacktestResult.logs`. There is no async log shipper — entries are in-memory until the orchestrator decides what to do with them.

---

## Plugin host

`src/plugin/` is a capability-gated extension point. A plugin loads via `Plugin::on_load` and receives a `PluginHost` — the host exposes one trait object per capability (MarketData, Execution, Storage, Indicators, Analytics, DSL extension, UI panels, Scheduler) plus an always-available `LogApi`. Each accessor has a `*_guarded` variant: plugins must declare the corresponding `Capability` in their manifest or the host returns `PluginError::PermissionDenied`.

Per-capability implementations live in `src/plugin/api/`:

- `market_data.rs` — `BrokerMarketDataApi` wraps `Arc<dyn BrokerClient>`. Subscriptions are tracked by `SubscriptionHandle` and backed by tokio `AbortHandle`s; `unsubscribe` calls `abort_handle.abort()` and returns `PluginError::NotFound` if the handle is missing.
- `storage.rs` — `PluginKvStore` is a per-plugin file-backed KV under `base_dir`. Keys are sanitized (`/`, `\`, `..`, `:` → `_`, truncated to 200 chars, empty → `_empty_`). Writes go through a `.tmp` file and `rename` for atomicity. All IO maps to `PluginError::ApiError`.
- `indicator_registry.rs` / `analytics.rs` — shared registries behind `parking_lot::RwLock`. Registrations carry a `PluginId`; a different plugin re-registering the same name gets `ApiError`, the same plugin gets a silent overwrite. `unregister_all_for` cleans up on plugin unload.
- `events.rs` — `EventBus` is a broadcast pub/sub: `subscribe(filter, callback)` pushes `(handle, filter, Arc<dyn Fn(EventKind) + Send + Sync>)` under an RW lock; `publish` collects matching callbacks under the read lock, drops the lock, then spawns a tokio task per callback to invoke it. The bus is wired into `StrategyEngine` via `event_bus: Option<Arc<EventBus>>` (default `None`); the engine publishes `RuleFired` (in the rule-eval pass on `should_fire == true`), `TradeExecuted` (in `submit_action` after `execute` returns `Ok`; the latest `PaperTrade` is recovered by downcasting the `Arc<dyn ExecutionTarget>` to `PaperBroker` via `as_any`), and `CandleProcessed` (after the cross-update pass). **Backtests leave `event_bus` as `None`** so plugin callbacks never fire during replay; the Tauri paper/live run sets it from a shared bus created in stage 9 (TODO marker in `src-tauri/src/main.rs`). `ExecutionTarget` exposes `as_any(&self) -> &dyn Any` so the engine can recover the concrete broker type for the `TradeExecuted` payload.
- `scheduler.rs` — `CronScheduler` parses cron expressions via the `cron` crate, sleeps to the next firing time with `tokio::time::sleep_until`, and uses `tokio_util::sync::CancellationToken` so `cancel` can interrupt the sleep without polling. Per-plugin tracking lives outside the scheduler in `PluginRegistry`.
- `log.rs` — `NamespacedLog` formats `[plugin:{id}] [{LEVEL}] {msg}` to stderr; logging is intentionally unguarded. Used by the CLI path.
- `log_file.rs` — `RateLimitedFileLog` is the production-grade `LogApi` implementation used by the Tauri host. It pairs a per-plugin token-bucket rate limiter (default 10 msg/sec burst, 100 msg/min sustained, shared across all log levels) with a 5MB rolling file under `<app_data>/logs/plugin-<id>.log`. Excess messages are silently dropped and a single per-minute summary line (`[plugin:<id>] [WARN] rate-limited: N message(s) dropped in the last 60s`) is written so a misbehaving plugin is visible without amplifying the spam. The file rotates on every write that would push the on-disk size past 5MB — the current file is renamed to `<base>.1` (older `*.1` overwritten) and a fresh current file is opened. Used by the Tauri host factory; the CLI does not load plugins and keeps using `NamespacedLog`.
- `ui.rs` — `TauriUiApi` keeps a `tokio::sync::broadcast::Sender<UiMessage>` (capacity 256). The Tauri layer holds the receiver and renders `PanelRegistered` / `Notification` / `PanelData` events.
- `dsl_extension.rs` — `SharedDslExtensionRegistry` is a `parking_lot::RwLock<HashMap<keyword, (PluginId, Arc<KeywordHandler>)>>`. The `DslExtensionApi` trait covers the keyword resolution surface the strategy engine calls during evaluation; `unregister_all_for(plugin_id)` lets the registry drop a plugin's keywords on disable/unload.
- `execution.rs` — `NoopExecutionApi` returns `PluginError::ApiError` from `submit_order` / `cancel_order` and an empty list from `positions`. It exists so the `Execution` capability slot has a real type until the strategy engine is wired to a broker-agnostic execution facade.

The plugin layer is wired into `PluginHostBuilder`. `PluginLoader::load_from_dir(dir)` (in `src/plugin/loader.rs`) reads `dir/plugin.toml`, derives `PluginMeta` and `Vec<Capability>` from it, and dispatches on the entry file's extension to either `RhaiPlugin::new` or `WasmPlugin::new` (passing the manifest's `permissions.max_memory_mb` to the WASM runtime). Unknown extensions yield `PluginError::LoadFailed`. `PluginRegistry` (in `src/plugin/registry.rs`) holds an `Arc<RwLock<HashMap<PluginId, PluginEntry>>>`; entries carry the boxed `Plugin`, the original `PluginManifest`, the current `PluginStatus`, and any `ScheduleHandle`s the plugin has armed. The registry is constructed with a `plugins_dir: PathBuf` and a `host_factory: Arc<dyn Fn(PluginId, Vec<Capability>, PluginPermissions) -> Arc<PluginHost> + Send + Sync>` so the host's wiring (broker handles, storage roots, UI broadcast sender, etc.) lives in the application, not in the plugin layer. `PluginRegistry::scan_and_load` walks the plugins directory, loads each subdirectory via the loader, builds a host via the factory, calls `plugin.on_load(host)`, and records `Loaded` (success) or `Failed(err)` (load error).

`enable` / `disable` / `unload` swap the real plugin out of the entry under the write lock (via an `EmptyPlugin` placeholder) before awaiting `on_enable` / `on_disable` / `on_unload`, then swap it back. This keeps the futures `Send` (parking_lot guards are `!Send` and holding one across `.await` would break Tauri's command dispatcher) and avoids deadlock if a plugin re-enters the registry during its callback. There is a small TOCTOU window between swap-out and swap-back, but `on_enable` / `on_disable` are idempotent for the plugins shipped in this repo and the registry is single-process.

**Plugin tests** (`src/plugin/tests.rs` and the per-module `#[cfg(test)] mod tests` blocks) cover storage, indicator-registry dedup, event-bus filter, manifest validation, the rate limiter (`src/plugin/api/log_file.rs::tests`), and the 5MB rolling log writer. The log-file tests cover: token-bucket admits-within-burst / blocks-after-burst / refill / sustained-window cap; rolling file rotates at the 5MB cap and appends to an existing file; the combined `RateLimitedFileLog` throttles a 50-msg spam run.

**Example plugin** (`strategies/example_plugin/`) is a reference Rhai plugin that demonstrates `Indicators` + `Storage` capabilities. `on_load` persists a monotonically-increasing load counter via `storage_set`/`storage_get` and registers a `double_ema` indicator (double EMA implemented in pure Rhai using `simple_ema`). `on_enable` / `on_disable` / `on_unload` log lifecycle events.

### Tauri wiring (`src-tauri/src/main.rs`)

The Tauri shell exposes four plugin commands and one Tauri-event channel:

| Command | Args | Body | Purpose |
|---|---|---|---|
| `list_plugins` | — | `commands::plugins::list_plugins(&state)` | Snapshot of every loaded plugin for the UI |
| `enable_plugin` | `id: String` | `commands::plugins::enable_plugin(&state, id)` | Move a loaded plugin into `Enabled` |
| `disable_plugin` | `id: String` | `commands::plugins::disable_plugin(&state, id)` | Move an enabled plugin back to `Disabled` |
| `reload_plugins` | — | `commands::plugins::reload_plugins(&state)` | Re-scan `plugins_dir`; returns per-plugin error messages |

Each `#[tauri::command]` wrapper is one line because the `tauri::command` macro generates module-private artifacts (`__cmd__name`, `__tauri_command_name_name`) that `tauri::generate_handler!` must resolve in the same scope — so the wrappers live in `main.rs` and the bodies live in the library.

`AppState` is defined in `src/commands/state.rs` and re-exported as `commands::AppState`. It carries `DataState`, `Arc<StrategyRegistry>`, `Arc<PluginRegistry>`, and a `tokio::sync::broadcast::Receiver<UiMessage>` for downstream consumers (e.g. a future audit-log command). The Tauri `setup` closure builds the plugin's shared infrastructure (registries, event bus, scheduler, broker wrappers, noop execution) and wires them into a single `HostFactory` closure that the registry calls per plugin. The factory also creates a `<app_data>/logs/` directory and hands each plugin a `RateLimitedFileLog` rooted there — see `src/plugin/api/log_file.rs`. After `scan_and_load`, a `tokio::spawn` subscribes a fresh `TauriUiApi` receiver and re-emits every `UiMessage` on the Tauri event bus as `"plugin-ui-message"` so the React frontend can subscribe once and dispatch on the `UiMessage` variant.

### Rhai plugin runtime (`src/plugin/runtime/rhai_runtime.rs`)

`RhaiPlugin` is a `Plugin` implementation that compiles a user-supplied `.rhai` source file with a heavily restricted `rhai::Engine` and invokes the script's `on_load` / `on_enable` / `on_disable` / `on_unload` functions at the corresponding lifecycle events.

**Engine hardening** — applied in `RhaiPlugin::new` before any plugin code runs:

- `set_max_operations(200_000)` — total op budget per script execution.
- `set_max_call_levels(32)` — recursion depth cap.
- `set_max_string_size(65_536)` / `set_max_array_size(10_000)` / `set_max_map_size(1_000)` — collection size caps.
- `on_print(|_| {})` — `print(...)` calls are silently swallowed.
- Module loading is intentionally NOT installed (no `set_module_resolver`), so plugins can only see what the host explicitly registers.

The `Candle` type is registered as a Rhai custom type `Candle` with getters for `open`, `high`, `low`, `close`, `volume`, `timestamp`.

**Host functions** — registered onto the engine inside `on_load` (so the engine `Arc` has a single strong count and we can use `Arc::get_mut` for `&mut Engine` access):

- `log_info` / `log_warn` / `log_error` — ungated; route through the host's `LogApi`. The Tauri host factory wires a `RateLimitedFileLog` so every call is checked against a per-plugin token-bucket and dropped (with a per-minute summary line) if the plugin is spamming; the CLI path keeps using `NamespacedLog` (no plugins are loaded from the CLI, so rate limiting is unnecessary).
- `storage_get(key)` / `storage_set(key, val)` — `Storage` capability; calls the underlying synchronous `StorageApi::read` / `write` and decodes `Vec<u8>` as UTF-8.
- `notify_info` / `notify_warning` / `notify_error` — `UiPanels` capability; emits a `Notification` over the UI broadcast channel.
- `register_indicator(name, fn_ptr)` — `Indicators` capability; the closure captures `Arc<Engine>` + `Arc<AST>` + a clone of the `FnPtr` and, on evaluation, dispatches the user's Rhai function with a `rhai::Array` of candle maps + the period. The trait-level `IndicatorRegistryApi` exposes a factory-based `register` that loses plugin-id information, so the runtime downcasts via `as_any()` back to the concrete `SharedIndicatorRegistry` and uses `register_fn` (which carries the `PluginId` for dedup). On any error or non-numeric return, the indicator pipeline receives a `Vec<f64>` of `NaN` of the same length as the input.

**Lifecycle wiring** — `RhaiPlugin::on_load` compiles `self.source_path` with `engine.compile_file`, registers all host functions, then `call_fn`s `on_load` (if defined). `EvalAltResult::ErrorFunctionNotFound` is swallowed; any other error maps to `PluginError::LoadFailed`. `on_enable` / `on_disable` follow the same pattern mapping to `PluginError::ApiError`. `on_unload` invokes the script's `on_unload` (errors ignored) and drops `self.host` and `self.ast`.

The engine and AST are stored in `Arc` so the `register_indicator` closure can hold long-lived references to them — Rhai's `Engine` is not `Clone`, so wrapping it in `Arc` is the only way to share it between the plugin struct and the registered host functions.

### WASM plugin runtime (`src/plugin/runtime/wasm_runtime.rs`)

`WasmPlugin` is a `Plugin` implementation that loads a `.wasm` artifact, links a small set of capability-gated host functions into the `algomln` module namespace, and invokes the exported `_algomln_on_load` / `_algomln_on_enable` / `_algomln_on_disable` / `_algomln_on_unload` functions at the corresponding lifecycle events.

**Engine configuration** — built eagerly in `WasmPlugin::new` from a `wasmtime::Config`:

- `async_support(false)` — synchronous execution, matches the rest of the engine.
- `epoch_interruption(true)` — the store's epoch deadline is armed to `1` on load, so a host-side watchdog can drive the engine's epoch counter and trap runaway plugins.
- `cranelift_opt_level(Speed)` — release-style codegen.
- Memory limit is computed from `memory_limit_mb * 1024 * 1024` and enforced by a `ResourceLimiter` (`MemoryLimitState`) that is stored inline in `WasmState` and handed to `store.limiter(|s: &mut WasmState| &mut s.memory_limiter)`. `memory_growing` returns `false` for any growth past the cap; `table_growing` caps tables at 10,000 entries.

**WASI is intentionally not linked.** `WasiCtx` in wasmtime 23 holds trait objects (`RngCore`, `HostWallClock`, `HostMonotonicClock`) that are `Send` but not `Sync`. Carrying a `WasiCtx` in `WasmState` would prevent `Store<WasmState>` from satisfying the `Sync` bound the `Plugin` trait requires, and therefore would prevent `WasmPlugin` from being `Sync` — which the rest of the host assumes. Plugins interact with the platform exclusively through the `algomln::*` host functions.

**Host functions** — bound in `build_linker`. All string/binary data crosses the WASM boundary through `(ptr, len)` pairs; helpers `read_string_from_memory` and `write_bytes_to_memory` decode/encode against the instance's `memory` export:

- `log_info(ptr, len)` / `log_warn(ptr, len)` / `log_error(ptr, len)` — ungated; route through the plugin's `LogApi` with the host's `PluginId` attached.
- `storage_get(key_ptr, key_len, out_ptr, out_len_ptr) -> i32` — `Storage` capability; returns `0` (write value at `out_ptr`, length at `out_len_ptr`), `1` (key not present, `out_len_ptr` set to 0), or `-1` (permission denied / IO error).
- `storage_set(key_ptr, key_len, val_ptr, val_len) -> i32` — `Storage` capability; returns `0` on success, `-1` on permission denied / IO error.
- `notify(msg_ptr, msg_len, kind)` — `UiPanels` capability; `kind` is `0` = Info, `1` = Warning, `2` = Error. Permission errors are logged but do not trap the instance.
- `emit_panel_data(panel_id_ptr, panel_id_len, json_ptr, json_len) -> i32` — `UiPanels` capability; the trait surface doesn't expose panel-data emission, so the implementation downcasts the `UiApi` to the concrete `TauriUiApi` via `as_any` and calls `emit_panel_data` so the broadcast channel picks the value up.

**Async bridge.** `StorageApi::read` / `write` are currently synchronous (the `async_trait` is forward-looking), but every host call still drives the work through `tokio::runtime::Handle::current().block_on(...)` so future async implementations compose without changing the WASM side.

**Lifecycle wiring** — `WasmPlugin::on_load` reads the artifact, compiles it with `Module::new`, builds the linker, constructs the store with the inline `MemoryLimitState`, sets the epoch deadline, instantiates, and dispatches `_algomln_on_load` if exported. `on_enable` / `on_disable` follow the same pattern for `_algomln_on_enable` / `_algomln_on_disable`. `on_unload` calls `_algomln_on_unload` (errors ignored) and drops both the store and the instance, releasing all memory back to wasmtime.