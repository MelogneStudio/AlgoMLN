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
│   │   └── dhan/               auth.rs / rest.rs / websocket.rs / models.rs
│   ├── models/                 Candle, Tick, Quote, Order, Position, Portfolio
│   ├── indicators/             Pure stateless fns: ema, ma, rsi, atr, vwap, bb, rel_vol
│   ├── feed/                   WebSocket feed manager (subscriptions, tick fan-out)
│   ├── data/                   Shared CSV loaders (load_nifty_candles, parse_market_row)
│   ├── strategy/
│   │   ├── dsl/                Lexer → Parser → AST → Validator
│   │   ├── runtime/            StrategyEngine + EvalContext, CrossDetector, TriggerStateMap
│   │   ├── execution/          ExecutionTarget trait; PaperBroker, order_builder
│   │   ├── logging/            StrategyLogger, LogEntry, LogEntryKind
│   │   ├── analytics.rs        BacktestAnalyser → BacktestSummary
│   │   └── tests/              Integration tests
│   ├── commands/               Tauri IPC command implementations
│   │   ├── data.rs             broker + feed wrappers
│   │   ├── strategy.rs         DSL helpers, backtest orchestrator, wire types
│   │   └── registry.rs         StrategyRegistry — JSON-persisted deploy/list/status
│   └── bin/
│       └── behavioral_backtest.rs   CLI runner (uses commands::strategy::run_backtest_internal)
│
├── src-tauri/                  Tauri v2 shell (re-exports the lib)
│   ├── tauri.conf.json         app identifier, window settings
│   ├── capabilities/           IPC permissions
│   └── src/main.rs             entrypoint: registers commands, opens registry, applies acrylic
│
├── src/                        React frontend root (TypeScript, Vite, React 19)
│   ├── main.tsx                mounts <App />, loads CSS tokens/fonts
│   ├── App.tsx                 top-level orchestrator: scale, screen/modal state, builder state
│   ├── App.module.css
│   ├── components/             AppWindow, TitleBar, Sidebar, Button, RuleRow, IndicatorPicker,
│   │                           NumberInput, OptionSlider, ScaleSlider
│   ├── screens/                Builder / Strategies / Settings / StrategyCoder / StrategyUploader
│   │   └── Builder/components/ BacktestPanel, RuleSection
│   ├── hooks/                  useStrategyBuilder, useDslSync, useBacktest
│   ├── lib/                    scaling.ts (DESIGN_WIDTH/HEIGHT, computeFitScale, applyScale)
│   └── types/                  tauri.ts (IPC wrappers + isTauri), strategy.ts, backtest.ts
│
├── sample-data/                bundled NIFTY 1-min CSV for offline backtests
├── strategies/                 sample .algomln files
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

### Execution / Brokers

| Concern | File |
|---|---|
| `ExecutionTarget` trait | `src/strategy/execution/target.rs` |
| Paper broker (in-memory cash + positions) | `src/strategy/execution/paper.rs` |
| `ActionNode` → `Order` builder | `src/strategy/execution/order_builder.rs` |
| BrokerClient trait (data fetch) | `src/broker/mod.rs` |
| Dhan auth / REST / WebSocket | `src/broker/dhan/{auth,rest,websocket,models}.rs` |
| Timeframe enum + Dhan interval strings | `src/broker/mod.rs` |

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
| Settings (default capital, about) | `src/screens/Settings/SettingsScreen.tsx` |
| Sidebar nav (Builder/Strategies/Settings) | `src/components/Sidebar/Sidebar.tsx` |

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
- **New Tauri command?** Implement in `src/commands/` (pick `data.rs` / `strategy.rs` / `registry.rs`), register with `#[tauri::command]` in `src-tauri/src/main.rs`, add to `invoke_handler!`.
- **New shared CSV loader?** Add to `src/data/csv.rs`; both the CLI and `commands::strategy::run_backtest_dsl` use it.
- **New field on a wire type?** Add the TS interface field in `src/types/`, add the Rust struct field (with `#[serde(rename_all = "camelCase")]`), and if it doesn't belong in `BacktestResult` (e.g. profile/log fields the UI doesn't render), extend `BacktestResultWire` in `src/commands/strategy.rs`.
- **New screen?** Add `src/screens/<Name>/<Name>Screen.tsx`, register it in `src/App.tsx` next to the existing `screen === 'builder'` branches. If it needs nav, add a `NavItem` entry in `src/components/Sidebar/Sidebar.tsx`.
- **New IPC command (frontend)?** Add the wrapper in `src/types/tauri.ts` using `invoke<T>(name, args)`, then call it from a hook. Always guard with `isTauri()` and provide a browser fallback so `npm run dev` stays demoable.
- **New builder rule field?** Update `BuilderRule` in `src/types/strategy.ts`, `strategyToDsl` / `parseDslToStrategy` in `src/hooks/useDslSync.ts`, and the visual controls in `src/components/RuleRow/`.
- **New button style?** Add a variant to `ButtonVariant` in `src/components/Button/Button.tsx` and a CSS class in `Button.module.css`.