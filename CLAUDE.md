# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

AlgoMLN is a local-first algorithmic trading platform built engine-first in Rust with a Tauri + React UI. The strategy engine is the heart of the project — a custom DSL compiles to an AST, which is evaluated deterministically candle-by-candle against historical (or live) data and routed to an `ExecutionTarget` (Paper or Live broker). Backtests, paper trading, and live trading share one engine — there is no separate "live code path."

The data flow is:
```
Data → Indicators → Strategy Engine → Backtesting → Execution → UI
```

## Common Commands

All commands are run from the repo root. Rust is the source of truth — UI is a thin client.

### Tests
```powershell
# Run full Rust test suite
cargo test --workspace

# Run only strategy tests
cargo test --lib strategy::

# Run a single test by name substring
cargo test --lib -- test_name_substring
```

### Backtest CLI
The `behavioral_backtest` binary runs any `.algomln` strategy against CSV data. Sample NIFTY 1-min data lives in `sample-data/`.

```powershell
# Build + run release binary
cargo run --release --bin behavioral_backtest -- run my_strategy.algomln --data sample-data/nifty_1min.csv --symbol NIFTY

# Limit candle count, override starting cash
cargo run --release --bin behavioral_backtest -- run my_strategy.algomln --data sample-data/nifty_1min.csv --candles 10000 --cash 500000

# Run a built-in named profile
cargo run --release --bin behavioral_backtest -- profile rsi 50000
```

### Tauri App
```powershell
# Install JS deps (first time)
npm install

# Run dev (Vite + Tauri together)
npm run tauri dev

# Frontend-only dev server (no Rust)
npm run dev

# Type-check + frontend production build
npm run build
```

### Linting / Type-check
There is no separate linter step. The frontend `build` script runs `tsc` which is the de-facto type check.

## High-Level Architecture

The Rust crate is at `src/` (not `src-tauri/src/` — that path is a Tauri shim that re-exports the lib). Layout:

```
src/
  broker/            BrokerClient trait + DhanClient implementation
    dhan/            auth.rs / rest.rs / websocket.rs / models.rs
  models/            Candle, Tick, Quote, Order, Position — all serde-serializable
  indicators/        Pure stateless functions: ema, ma, rsi, atr, vwap, bb (bollinger), rel_vol
  feed/              WebSocket manager — up to 1,000 symbol subscriptions, auto-reconnect, tick fan-out
  strategy/
    dsl/             Compiler pipeline: lexer → parser → ast → validator
    runtime/         StrategyEngine — candle-by-candle evaluation loop
                     EvalContext, CrossDetector, TriggerStateMap,
                     IndicatorProvider, BoundedWindowProvider, IncrementalIndicatorProvider
    execution/       ExecutionTarget trait; PaperBroker, DhanBroker; order_builder
    logging/         StrategyLogger, LogEntry, LogEntryKind
    analytics.rs     Backtest result metrics
    tests/           Integration tests (backtest_integration.rs, etc.)
  commands/          Tauri IPC commands — data.rs (broker/data), strategy.rs (backtests/deploy),
                     mod.rs; the bridge Rust → React
  bin/behavioral_backtest.rs   CLI backtest runner (uses commands::strategy::run_backtest_internal)
src-tauri/
  src/main.rs        Tauri app entrypoint — registers commands, loads .env, sets up DataState
```

## Frontend Architecture (React + Tauri)

Stack: **Vite + React 19 + TypeScript**, run inside a Tauri v2 webview. The Rust crate is the source of truth — the React layer is a thin client that issues IPC calls and renders results. There is no separate "live code path" in the UI either; paper and live deploys share the same screens and hooks.

### Layout

```
src/                       React frontend root (TypeScript, Vite, React 19)
  main.tsx                 Mounts <App /> into #root, loads global CSS tokens/fonts
  App.tsx                  Top-level orchestrator — owns scale, screen/modal state, builder state,
                           and wires BuilderScreen / StrategiesScreen / SettingsScreen /
                           StrategyCoderScreen / StrategyUploaderScreen together
  App.module.css           Layout for App shell
  components/
    AppWindow/             Root shell; injects --ui-scale CSS variable
    TitleBar/              Custom title bar with minimize/close + tauri-drag-region
    Sidebar/               Hard-locked nav (Builder / Strategies / Settings);
                           forced-collapsed at scale < 0.75
    Button/                Single Button primitive with variants: primary | ghost | code
    RuleRow/               One row of the strategy builder (indicator + period + op + rhs + action)
    IndicatorPicker/       Dropdown for IndicatorKind
    NumberInput/           Numeric input control
    OptionSlider/          Reusable slider
    ScaleSlider/           Used in Settings for --ui-scale
  screens/
    Builder/               Main visual strategy builder. Hosts RuleSection (entry/exit) +
                           BacktestPanel. Subcomponents live in components/.
    Builder/components/    BacktestPanel (summary + trade table), RuleSection (entry/exit wrapper)
    Strategies/            List of deployed strategies (Tauri IPC: list_strategies). Shows
                           DEMO_STRATEGIES constant when running outside Tauri (npm run dev).
    Strategies/components/ StrategyCard
    StrategyCoder/         Modal textarea editor for raw .algomln source. Handles Tab→2-space,
                           Esc→close. Read-only mode for viewing deployed strategies.
    StrategyUploader/      Modal for uploading/loading .algomln files from disk
    Settings/              Interface scale slider, default backtest capital, about card
  hooks/
    useStrategyBuilder     Builder state (entry rule, exit rule, advanced mode).
                           loadFromDsl() round-trips a `.algomln` string back into the builder
                           shape; if the DSL has features the builder can't represent (multiple
                           rules, AND/OR, cross_*), it returns false and toggles advanced mode
                           so the user falls back to the coder.
    useDslSync             Derives a live DSL string from builder state via strategyToDsl(),
                           then debounces validateDsl() IPC calls by 500ms. In the browser
                           fallback it always reports valid (the builder only emits grammars
                           we can construct locally).
    useBacktest            Calls runBacktest IPC; synthesizes a benign empty BacktestResult
                           when isTauri() is false so the UI is still demoable.
  lib/
    scaling.ts             DESIGN_WIDTH/HEIGHT (1550x757), computeFitScale(), applyScale()
                           (calls Tauri win.setSize on LogicalSize), localStorage persistence
                           keys algomln_ui_scale and algomln_default_capital
  types/
    tauri.ts               Thin IPC wrappers + isTauri() detection (checks
                           '__TAURI_INTERNALS__' in window). Functions: runBacktest,
                           deployStrategy, setStrategyStatus, listStrategies, validateDsl.
    strategy.ts            BuilderRule, BuilderStrategy, DeployedStrategy, IndicatorKind,
                           INDICATOR_DISPLAY map, INDICATOR_ORDER
    backtest.ts            BacktestResult, BacktestSummary, PaperTrade
```

### Frontend ↔ Rust Contract

- Rust models serialize to camelCase JSON. TypeScript mirrors live in `src/types/` (e.g. `types/strategy.ts`, `types/backtest.ts`, `types/tauri.ts`).
- IPC calls go through `@tauri-apps/api` `invoke()` — the canonical wrappers are in `src/types/tauri.ts`. See `src/hooks/useBacktest.ts` for the canonical pattern (call → set loading → try/catch → set result or error → finally clear loading).
- All Tauri-only flows (real backtests, live strategy list) MUST guard with `isTauri()` and provide a browser fallback so `npm run dev` (frontend only) is still demoable. Examples: `useBacktest` synthesizes an empty result, `StrategiesScreen` shows `DEMO_STRATEGIES`, `useDslSync` skips validation.
- Charts use `lightweight-charts` (TradingView). Candle data is piped Rust → React via Tauri IPC commands.
- Title bar uses `data-tauri-drag-region` to enable native drag on the Tauri webview; the right-side control buttons opt out (`data-tauri-drag-region={false}`).

### Builder ↔ Coder Round-trip

- `useStrategyBuilder` owns `BuilderStrategy` (a constrained shape: one entry rule + one exit rule, each with a single indicator comparison).
- `useDslSync` derives a `.algomln` string from that state with `strategyToDsl()` and validates it via IPC.
- `useStrategyBuilder.loadFromDsl(dsl)` parses DSL back via `parseDslToStrategy()`. If the DSL contains features the visual builder can't represent (multiple rules, AND/OR, cross_above/cross_below), parsing returns `null`, the hook flips `isAdvancedMode=true`, and the user stays in the Strategy Coder for editing.
- When the user clicks "Open Strategy Coder" from the builder, the live DSL is preloaded into the editor. When they click "Done", the new source goes back through `loadFromDsl`.

### Scaling

The UI is designed at a fixed 1550×757 logical canvas. `AppWindow` injects `--ui-scale` as a CSS variable; the entire shell scales by that factor. `applyScale()` resizes the OS window to match via `Tauri Window.setSize(LogicalSize)`. Below scale 0.75, the sidebar is **force-collapsed** (toggle button hidden) — see `SIDEBAR_FORCE_COLLAPSE_THRESHOLD` in `src/lib/scaling.ts`. Scale only changes via the Settings slider; the app does NOT auto-rescale on window resize (a prior 2-second poll caused feedback loops with Tauri's setSize and was removed — see the comment in `src/App.tsx`).

### Where to Add What (UI)

- **New screen?** Add `src/screens/<Name>/<Name>Screen.tsx`, register it in `src/App.tsx` next to the existing `screen === 'builder'` branches. If it needs nav, add a `NavItem` entry in `src/components/Sidebar/Sidebar.tsx`.
- **New IPC command?** Add the wrapper in `src/types/tauri.ts` using `invoke<T>(name, args)`, then call it from a hook. Register the Rust side in `src-tauri/src/main.rs` with `#[tauri::command]` and the implementation in `src/commands/`.
- **New rule field?** Update `BuilderRule` in `src/types/strategy.ts`, `strategyToDsl` / `parseDslToStrategy` in `src/hooks/useDslSync.ts`, and the visual controls in `src/components/RuleRow/`.
- **New button style?** Add a variant to `ButtonVariant` in `src/components/Button/Button.tsx` and a CSS class in `Button.module.css`.

## Critical Architecture Invariants

These are non-obvious properties the codebase is built around. Don't break them.

**1. Determinism.** Same strategy + same candles = identical output, every run. There is a dedicated determinism test for this. The rule is: `BTreeMap` over `HashMap` anywhere iteration order could affect evaluation. No randomness, no non-deterministic data structures on eval paths. The engine uses `BTreeMap`-backed registries.

**2. Trigger state machine.** A bare `WHEN x > y / BUY 1` would fire every candle. The `TriggerStateMap` fires rules only on `false → true` transitions. Crossovers use `CrossDetector`, which stores `(fast_prev, slow_prev)` and fires on the exact transition candle. Critically, the cross update pass runs **after** all rules are evaluated for the cycle — not inside the rule loop — so all rules see consistent previous-candle state within a single cycle.

**3. One execution engine.** Backtests, paper, and live all run through the same `StrategyEngine` and `ExecutionTarget` trait. The engine never knows which broker is behind the target. `PaperBroker` and `DhanBroker` are interchangeable implementations.

**4. Indicator windows are bounded.** `BoundedWindowProvider` keeps indicators O(N) over the backtest — 184,863 NIFTY 1-min candles complete in ~3.5s (~52k candles/sec). Don't refactor this into naive full-history recomputation.

**5. DSL grammar is intentionally small.** Rules only, no variables, no loops. Keywords case-insensitive. Indicator periods and quantities must be positive integers. `position_expr` and `time_window` parse but are not yet evaluated at runtime — see `src/strategy/dsl/ast.rs` and the corresponding `NotYetImplemented` branch in the evaluator.

## The `.algomln` DSL

Strategies live in `.algomln` files. Grammar (keywords case-insensitive):

```
strategy       = rule+
rule           = "WHEN" condition NEWLINE action
condition      = comparison | cross_expr | not_expr | logical_expr
                       | position_expr | time_window
comparison     = expr operator expr        (op in <, >, <=, >=, ==, !=)
logical_expr   = condition "AND" condition | condition "OR" condition
not_expr       = "NOT" "(" condition ")"
cross_expr     = cross_above(expr, expr) | cross_below(expr, expr)
expr           = indicator_call | price_field | number
indicator      = ema | ma | rsi | rel_vol | atr | vwap
               | bb_upper | bb_lower | bb_mid
price_field    = close | open | high | low | volume
               | prev_close | prev_open | prev_high | prev_low
action         = BUY <int> | SELL <int> | SELL ALL
```

Blank lines and `# comments` allowed anywhere. Examples are in the `strategies/` directory and the README.

## Tauri IPC

`src-tauri/src/main.rs` is a thin shell. The actual command implementations live in `src/commands/`. Each is a free function taking `&DataState` (or similar) — the Tauri wrapper just unwraps `State<'_, AppState>` and forwards. The CLI binary `behavioral_backtest` calls `run_backtest_internal` directly to avoid spawning a Tauri runtime.

**Env requirement:** Tauri app requires `DHAN_ACCESS_TOKEN` in `.env` (see `.env.example`). The CLI also loads `.env` via `dotenvy::dotenv()`.

## Where to Add What

- **New indicator?** Add a pure `fn name(candles: &[Candle], period: usize) -> Vec<f64>` in `src/indicators/`, register it in `src/indicators/mod.rs`, add the AST `IndicatorKind` variant in `src/strategy/dsl/ast.rs`, add a parser token in `lexer.rs`, and an evaluator branch in the engine.
- **New broker?** Implement `BrokerClient` (in `src/broker/`) and `ExecutionTarget` (in `src/strategy/execution/`). The engine needs no changes.
- **New DSL keyword?** Lexer → parser → AST → validator → engine evaluator — five files in `src/strategy/dsl/` and `src/strategy/runtime/`. Mirror the existing `cross_above` / `cross_below` flow as the closest reference.
- **New Tauri command?** Add the implementation in `src/commands/`, register it in `src-tauri/src/main.rs` with `#[tauri::command]`.

## Test Layout

- Per-module unit tests live alongside source as `#[cfg(test)] mod tests` blocks.
- Strategy integration tests are in `src/strategy/tests/` (notably `backtest_integration.rs`).
- A dedicated determinism test guards the same-input-same-output property.
- No frontend test suite is wired up — verify UI changes manually via `npm run tauri dev`.
