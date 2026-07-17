# ARCHITECTURE.md

File tree and "where to look for X" lookup tables. For *why* the layout is the way it is, read BACKEND.md and FRONTEND.md.

> **Conventions**
> - The Rust crate lives at `src/` (not `src-tauri/src/` — that path is a Tauri shim that re-exports the lib).
> - Wire types crossing the IPC boundary carry `#[serde(rename_all = "camelCase")]`. Internal Rust types do not.
> - React app is at the repo-root `src/` — yes, it overlaps with the Rust `src/`. Build tools disambiguate.

---

## Repository Layout

```
AlgoMLN/
├── Cargo.toml                  workspace manifest (binary + library)
├── package.json                Vite + React frontend
├── src/                        Rust library crate (source of truth)
│   ├── lib.rs                  module declarations
│   ├── broker/                 BrokerClient trait + DhanClient impl
│   │   ├── dhan/               auth.rs / rest.rs / websocket.rs / models.rs
│   │   └── symbol_map.rs       NSE symbol → Dhan SECURITY_ID map (seed + refresh)
│   ├── models/                 Candle, Tick, Quote, Order, Position, Portfolio
│   ├── indicators/             Pure stateless fns: ema, ma, rsi, atr, vwap, bb, rel_vol
│   ├── indices/                NSE index constituent registry (read-only after load)
│   │   ├── mod.rs              module root
│   │   ├── registry.rs         IndexRegistry (parking_lot RwLock, cache+resource dirs)
│   │   └── refresh.rs          refresh_index, refresh_all_if_stale (niftyindices.com)
│   ├── feed/                   WebSocket feed manager (subscriptions, tick fan-out)
│   ├── data/                   Shared CSV loaders (load_nifty_candles, parse_market_row)
│   ├── strategy/
│   │   ├── dsl/                Lexer → Parser → AST → Validator
│   │   ├── runtime/            StrategyEngine + EvalContext, CrossDetector, TriggerStateMap
│   │   ├── execution/          ExecutionTarget trait; PaperBroker, order_builder
│   │   ├── logging/            StrategyLogger, LogEntry, LogEntryKind
│   │   ├── analytics.rs        BacktestAnalyser → BacktestSummary
│   │   └── tests/              Integration tests
│   ├── plugin/                 Plugin host + capability-gated APIs
│   │   ├── api/                Trait definitions + per-capability implementations
│   │   │   ├── mod.rs          MarketData/Storage/Indicator/Analytics/Ui/Scheduler/Log traits
│   │   │   ├── market_data.rs  BrokerMarketDataApi — wraps BrokerClient
│   │   │   ├── storage.rs      PluginKvStore — per-plugin sandboxed file KV
│   │   │   ├── indicator_registry.rs  SharedIndicatorRegistry (plugin-id dedup)
│   │   │   ├── analytics.rs    SharedAnalyticsRegistry (plugin-id dedup)
│   │   │   ├── dsl_extension.rs  SharedDslExtensionRegistry (DSL keyword registration)
│   │   │   ├── events.rs       EventBus + EventKind + EventFilter (pub/sub)
│   │   │   ├── execution.rs    NoopExecutionApi — stub until wired into engine
│   │   │   ├── scheduler.rs    CronScheduler — cron + CancellationToken per task
│   │   │   ├── log.rs          NamespacedLog — eprintln! gated by plugin_id (CLI)
│   │   │   ├── log_file.rs     RateLimitedFileLog — token-bucket rate limit + 5MB rolling file per plugin
│   │   │   └── ui.rs           TauriUiApi — broadcast channel for UI panels
│   │   ├── host.rs             PluginHost (capability-gated accessors) + Builder
│   │   ├── loader.rs           PluginLoader — manifest → boxed Plugin (rhai/wasm)
│   │   ├── manifest.rs         PluginManifest + PluginPermissions
│   │   ├── registry.rs         PluginRegistry — in-memory map + lifecycle + host factory
│   │   ├── runtime/            Plugin language runtimes
│   │   │   ├── rhai_runtime.rs RhaiPlugin — Rhai script compilation + host fns
│   │   │   └── wasm_runtime.rs WasmPlugin — wasmtime module + `algomln::*` host fns
│   │   ├── types.rs            PluginId, PluginMeta, Capability, PluginError, handles
│   │   ├── tests.rs            Plugin unit tests (storage, indicator registry, event bus, manifest)
│   │   └── mod.rs              Plugin trait, plugin module root
│   ├── commands/               Tauri IPC command implementations
│   │   ├── data.rs             broker + feed wrappers
│   │   ├── strategy.rs         DSL helpers, backtest orchestrator, wire types
│   │   ├── registry.rs         StrategyRegistry — JSON-persisted deploy/list/status
│   │   ├── state.rs            AppState — the struct held by Tauri::manage
│   │   └── plugins.rs          list/enable/disable/reload plugin command bodies
│   └── bin/
│       └── behavioral_backtest.rs   CLI runner (uses commands::strategy::run_backtest_internal)
│
├── src-tauri/                  Tauri v2 shell (re-exports the lib)
│   ├── tauri.conf.json         app identifier, window settings, bundled resources
│   ├── capabilities/           IPC permissions
│   ├── resources/              bundled resources (seed index JSON, icons)
│   │   └── indices/            seed NSE index constituent JSON (gitkeep; populated by `scripts/fetch_seed_indices.py`)
│   └── src/main.rs             entrypoint: registers commands, opens registry, applies acrylic
│
├── src/                        React frontend root (TypeScript, Vite, React 19)
│   ├── main.tsx                mounts <App />, loads CSS tokens/fonts
│   ├── App.tsx                 top-level orchestrator: scale, screen/modal state, builder state
│   ├── App.module.css
│   ├── components/             AppWindow, TitleBar, Sidebar, Button, RuleRow, IndicatorPicker,
│   │                           NumberInput, OptionSlider, ScaleSlider
│   ├── screens/                Builder / Strategies / Plugins / Settings / StrategyCoder / StrategyUploader
│   │   └── Builder/components/ BacktestPanel, RuleSection
│   ├── hooks/                  useStrategyBuilder, useDslSync, useBacktest
│   ├── lib/                    scaling.ts (DESIGN_WIDTH/HEIGHT, computeFitScale, applyScale)
│   └── types/                  tauri.ts (IPC wrappers + isTauri), strategy.ts, backtest.ts, plugin.ts
│
├── sample-data/                bundled NIFTY 1-min CSV for offline backtests
├── strategies/                 sample .algomln files + plugin examples
│   └── example_plugin/         reference Rhai plugin (double_ema indicator, load-count storage)
├── plans/                      design notes (data layer, scripting, runtime, etc.)
└── .env.example                DHAN_ACCESS_TOKEN template
```

---

## Lookup Tables — "Where Do I Look For…"

### DSL (`.algomln` strategy language)

| Concern | File |
|---|---|
| Lexer (tokens, keywords, errors) | `src/strategy/dsl/lexer.rs` |
| Parser (token stream → AST) | `src/strategy/dsl/parser.rs` |
| AST types (`StrategyNode`, `RuleNode`, `ConditionNode`, `ExprNode`, `IndicatorKind`, `PriceField`, `CompareOp`, `ActionNode`) | `src/strategy/dsl/ast.rs` |
| Semantic validation (period > 0, qty > 0, duplicate rule IDs, time range, etc.) | `src/strategy/dsl/validator.rs` |
| DSL → `BuilderStrategy` round-trip (frontend only) | `src/hooks/useDslSync.ts` (`strategyToDsl`, `parseDslToStrategy`) |
| Grammar spec | `CLAUDE.md` "The `.algomln` DSL" (this codebase keeps grammar in the index; mirror any grammar changes there) |

### Strategy engine / runtime

| Concern | File |
|---|---|
| Candle-by-candle evaluation loop | `src/strategy/runtime/engine.rs` (`StrategyEngine::on_candle`) |
| Per-rule evaluation context | `src/strategy/runtime/context.rs` (`EvalContext`) |
| Trigger state machine (false → true) | `src/strategy/runtime/trigger_state.rs` (`TriggerStateMap`) |
| Crossover detection | `src/strategy/runtime/cross.rs` (`CrossDetector`) |
| Bounded indicator window (perf) | `src/strategy/runtime/incremental_provider.rs` (`BoundedWindowProvider`) |
| Full-recompute indicator provider (test/benchmark) | `src/strategy/runtime/indicator_provider.rs` (`FullRecomputeProvider`) |
| Backtest analytics (win rate, drawdown, etc.) | `src/strategy/analytics.rs` (`BacktestAnalyser`, `BacktestSummary`) |
| Engine log entries | `src/strategy/logging/log.rs` |

### Indicators

| Concern | File |
|---|---|
| Moving averages (ma, ema) | `src/indicators/ma.rs` |
| RSI | `src/indicators/rsi.rs` |
| ATR | `src/indicators/atr.rs` |
| VWAP | `src/indicators/vwap.rs` |
| Bollinger bands | `src/indicators/bb.rs` |
| Relative volume | `src/indicators/mod.rs::rel_vol` |
| Indicator dispatch (latest value) | `src/strategy/runtime/incremental_provider.rs::latest_indicator_value` |

### Plugin host

| Concern | File |
|---|---|
| Capability traits (MarketData/Storage/Indicator/Analytics/Ui/Scheduler/Log) | `src/plugin/api/mod.rs` |
| Per-capability implementations | `src/plugin/api/{market_data,storage,indicator_registry,analytics,events,scheduler,log,ui}.rs` |
| Capability gating + `*_guarded` accessors | `src/plugin/host.rs` (`PluginHost`, `PluginHostBuilder`) |
| Plugin identity, errors, handles | `src/plugin/types.rs` |
| Plugin lifecycle trait | `src/plugin/mod.rs` (`Plugin`) |
| Plugin manifest + permissions | `src/plugin/manifest.rs` (`PluginManifest`, `PluginPermissions`) |
| Plugin loader (manifest → boxed Plugin) | `src/plugin/loader.rs` (`PluginLoader::load_from_dir`) |
| Plugin registry (in-memory map, lifecycle, host factory) | `src/plugin/registry.rs` (`PluginRegistry`) |
| Rhai script runtime (engine budgets, host fns, lifecycle) | `src/plugin/runtime/rhai_runtime.rs` (`RhaiPlugin`) |
| WASM plugin runtime (wasmtime, capability-gated host fns) | `src/plugin/runtime/wasm_runtime.rs` |
| Broadcast pub/sub for plugin subscribers (no engine coupling) | `src/plugin/api/events.rs` (`EventBus`, `EventKind`) |
| Engine event-bus hook (publishes `RuleFired` / `TradeExecuted` / `CandleProcessed` from `on_candle`) | `src/strategy/runtime/engine.rs` (`StrategyEngine::event_bus`, `latest_paper_trade`) |
| DSL keyword registration (plugin-extensible AST handlers) | `src/plugin/api/dsl_extension.rs` (`SharedDslExtensionRegistry`) |
| Execution capability stub (rejects orders until wired into engine) | `src/plugin/api/execution.rs` (`NoopExecutionApi`) |

### Plugin IPC (Tauri side)

The Tauri binary wires the plugin layer to the desktop shell at startup
(see `src-tauri/src/main.rs::main`):

1. **Shared infrastructure** is built once inside the `setup` closure
   and `Arc`-cloned into every host the registry creates. This means a
   registration made by one plugin is visible to the engine and to
   other plugins:

   | Resource | What the plugin sees |
   |---|---|
   | `SharedIndicatorRegistry` | `register(name, fn)` and `get(name)` (mutex-guarded map) |
   | `SharedAnalyticsRegistry` | `register_metric(name, fn)` and `get_metric(name)` |
   | `SharedDslExtensionRegistry` | `register_keyword(name, handler)` (DSL keyword plugins can resolve) |
   | `EventBus` | `publish` / `subscribe` (candle + trade + rule + status events) |
   | `TauriUiApi` | `register_panel` / `notify` / `emit_panel_data` (broadcast to the Tauri bus) |
   | `CronScheduler` | `schedule(cron, task)` / `cancel(handle)` |
   | `BrokerMarketDataApi` | wraps the same `DhanClient` the strategy layer uses |
   | `NoopExecutionApi` | stub — `submit_order` returns `ApiError` until a future revision wires a real broker adapter |
   | `PluginKvStore` | per-plugin sandboxed file KV under `<app_data>/plugins/<id>/storage` |
   | `NamespacedLog` | `eprintln!` gated by plugin id (CLI path) |
   | `RateLimitedFileLog` | per-plugin token-bucket (10/sec burst, 100/min) + 5MB rolling file under `<app_data>/logs/plugin-<id>.log` (Tauri path) |

2. **Host factory** is a single `Arc<HostFactory>` closure bound to the
   `plugins_dir` path. The registry calls it for every plugin it
   discovers, with the plugin's declared `Capability` set and
   `PluginPermissions` from its `plugin.toml`.

3. **Load + scan** runs once at startup via
   `tauri::async_runtime::block_on(registry.scan_and_load())`. Each
   `plugin.toml`-bearing subdirectory of `<app_data>/plugins/` is
   loaded; per-plugin load results are logged to `stderr`.

4. **UI forwarder** is a `tokio::spawn` that subscribes a fresh
   `broadcast::Receiver<UiMessage>` from the `TauriUiApi` and re-emits
   every message on the Tauri event bus as `"plugin-ui-message"`. The
   React app subscribes once and dispatches on the `UiMessage` variant
   (`PanelRegistered` / `Notification` / `PanelData`).

5. **Tauri commands** (defined in `src-tauri/src/main.rs`, delegating
   to the plain-async bodies in `src/commands/plugins.rs`):

   | Command | Args | Returns |
   |---|---|---|
   | `list_plugins` | — | `Vec<PluginListEntry>` (id, name, status, capabilities) |
   | `enable_plugin` | `id: String` | `()` |
   | `disable_plugin` | `id: String` | `()` |
   | `reload_plugins` | — | `Vec<String>` (per-plugin error messages, empty = clean) |

   `#[tauri::command]` wrappers are kept in `main.rs` because the
   macro generates module-private artifacts (`__cmd__name`,
   `__tauri_command_name_name`) that `tauri::generate_handler!` must
   be able to resolve in the same scope.

6. **Lifecycle lock discipline**: `PluginRegistry::enable`,
   `disable`, and `unload` swap the real plugin out of the entry
   under the write lock, then call `on_enable` / `on_disable` /
   `on_unload` outside the lock, then swap it back. This keeps the
   futures `Send` (parking_lot guards are `!Send`) and avoids
   deadlock if a plugin re-enters the registry during its callback.

### Execution / Brokers

| Concern | File |
|---|---|
| `ExecutionTarget` trait | `src/strategy/execution/target.rs` |
| Paper broker (in-memory cash + positions) | `src/strategy/execution/paper.rs` |
| `ActionNode` → `Order` builder | `src/strategy/execution/order_builder.rs` |
| BrokerClient trait (data fetch) | `src/broker/mod.rs` |
| Dhan auth / REST / WebSocket | `src/broker/dhan/{auth,rest,websocket,models}.rs` |
| Timeframe enum + Dhan interval strings | `src/broker/mod.rs` |

### Indices & symbol resolution (multi-symbol strategies)

| Concern | File |
|---|---|
| `IndexAlias` enum (22 NSE indices) + `TradeIn` AST | `src/strategy/dsl/ast.rs` |
| `TRADE_IN` keyword lex + parse | `src/strategy/dsl/{lexer,parser}.rs` |
| `IndexRegistry` (read-only after load) | `src/indices/registry.rs` |
| Index refresh from niftyindices.com | `src/indices/refresh.rs` |
| Seed fetcher (Python, stdlib only) | `scripts/fetch_seed_indices.py` |
| Bundled seed JSON | `src-tauri/resources/indices/*.json` |
| NSE symbol → Dhan `SECURITY_ID` map | `src/broker/symbol_map.rs` |
| Dhan scrip master CSV refresh | `src/broker/symbol_map.rs::refresh_symbol_map` |

### Tauri IPC

| Concern | File |
|---|---|
| Tauri command handlers (one-liners → library fns) | `src-tauri/src/main.rs` |
| `invoke_handler!` registration list | `src-tauri/src/main.rs` (search `tauri::generate_handler!`) |
| Data commands (OHLCV, quote, ticks) | `src/commands/data.rs` |
| Backtest orchestrator + wire types | `src/commands/strategy.rs` (`run_backtest_dsl`, `validate_dsl`, `BacktestResultWire`, `PaperTradeWire`) |
| Strategy registry (deploy/list/set_status) | `src/commands/registry.rs` |
| Registry persistence path | `%APPDATA%\com.algomln.app\strategies.json` on Windows (`app_data_dir` + `strategies.json`) |

### Wire types & IPC contract

| Concern | File |
|---|---|
| `BacktestResult`, `BacktestSummary`, `BacktestProfile`, `EngineProfileReport`, `IndicatorProfileReport` (internal) | `src/commands/strategy.rs` |
| `BacktestResultWire`, `PaperTradeWire` (Tauri-facing) | `src/commands/strategy.rs` |
| TS mirror of `BacktestResult` | `src/types/backtest.ts` |
| TS mirror of `DeployedStrategy` | `src/types/strategy.ts` |
| IPC wrapper functions | `src/types/tauri.ts` |
| `isTauri()` detection (browser fallback gate) | `src/types/tauri.ts` |

### Frontend

| Concern | File |
|---|---|
| App shell + scale + screen/modal state | `src/App.tsx` |
| UI scale (DESIGN_WIDTH/HEIGHT = 1550×757) | `src/lib/scaling.ts` |
| Builder state (entry + exit rule) | `src/hooks/useStrategyBuilder.ts` |
| Live DSL ↔ builder sync + IPC validation | `src/hooks/useDslSync.ts` |
| Backtest hook (calls `run_backtest`, browser fallback) | `src/hooks/useBacktest.ts` |
| Visual builder screen | `src/screens/Builder/BuilderScreen.tsx` |
| Builder rule row (one indicator comparison) | `src/components/RuleRow/RuleRow.tsx` |
| DSL editor modal | `src/screens/StrategyCoder/StrategyCoderScreen.tsx` |
| File upload modal | `src/screens/StrategyUploader/StrategyUploaderScreen.tsx` |
| Deployed strategies list | `src/screens/Strategies/StrategiesScreen.tsx` |
| Plugin management (list/enable/disable/reload, DEMO_PLUGINS fallback) | `src/screens/Plugins/PluginsScreen.tsx` |
| Settings (default capital, about) | `src/screens/Settings/SettingsScreen.tsx` |
| Plugin wire types (`PluginListEntry`, `PluginMeta`, `PluginStatus`, `Capability`) | `src/types/plugin.ts` |
| Sidebar nav (Builder/Strategies/Plugins/Settings) | `src/components/Sidebar/Sidebar.tsx` |

### Data

| Concern | File |
|---|---|
| NIFTY CSV loader | `src/data/csv.rs::load_nifty_candles` |
| CSV row parser (tab/comma/whitespace) | `src/data/csv.rs::parse_market_row` |
| CLI candle loader (legacy tiny format) | `src/bin/behavioral_backtest.rs::load_tiny_candles` |

### CLI

| Concern | File |
|---|---|
| CLI entrypoint + subcommands (`run`, `profile`, `backtest`) | `src/bin/behavioral_backtest.rs` |
| Built-in profiles (`rsi`, `ema`) | `src/bin/behavioral_backtest.rs::run_profile` |
| CLI sample data default | `sample-data/nifty_1min.csv` |

---

## "Where to Add…" Recipes

- **New indicator?** Pure `fn name(candles: &[Candle], period: usize) -> Vec<f64>` in `src/indicators/`, register in `src/indicators/mod.rs`, add AST variant in `src/strategy/dsl/ast.rs`, add a parser token in `lexer.rs`, add an evaluator branch in the engine.
- **New broker?** Implement `BrokerClient` in `src/broker/` and `ExecutionTarget` in `src/strategy/execution/`. The engine needs no changes.
- **New DSL keyword?** Lexer → parser → AST → validator → engine evaluator (five files in `src/strategy/dsl/` + `src/strategy/runtime/`). Mirror `cross_above` / `cross_below` as the closest reference.
- **New Tauri command?** Implement the body as a plain `async fn` in `src/commands/` (pick `data.rs` / `strategy.rs` / `registry.rs` / `plugins.rs` / or add a new file), then add a thin `#[tauri::command]` wrapper in `src-tauri/src/main.rs` that delegates, and add the wrapper to `invoke_handler!`. `AppState` is defined in `src/commands/state.rs` and re-exported as `commands::AppState` so command bodies can use it without depending on the binary crate.
- **New shared CSV loader?** Add to `src/data/csv.rs`; both the CLI and `commands::strategy::run_backtest_dsl` use it.
- **New field on a wire type?** Add the TS interface field in `src/types/`, add the Rust struct field (with `#[serde(rename_all = "camelCase")]`), and if it doesn't belong in `BacktestResult` (e.g. profile/log fields the UI doesn't render), extend `BacktestResultWire` in `src/commands/strategy.rs`.
- **New screen?** Add `src/screens/<Name>/<Name>Screen.tsx`, register it in `src/App.tsx` next to the existing `screen === 'builder'` branches. If it needs nav, add a `NavItem` entry in `src/components/Sidebar/Sidebar.tsx`.
- **New IPC command (frontend)?** Add the wrapper in `src/types/tauri.ts` using `invoke<T>(name, args)`, then call it from a hook. Always guard with `isTauri()` and provide a browser fallback so `npm run dev` stays demoable.
- **New builder rule field?** Update `BuilderRule` in `src/types/strategy.ts`, `strategyToDsl` / `parseDslToStrategy` in `src/hooks/useDslSync.ts`, and the visual controls in `src/components/RuleRow/`.
- **New button style?** Add a variant to `ButtonVariant` in `src/components/Button/Button.tsx` and a CSS class in `Button.module.css`.