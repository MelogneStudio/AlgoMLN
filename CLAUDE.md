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
  broker/            BrokerClient trait + DhanClient implementation (auth, rest, websocket, models)
  models/            Candle, Tick, Quote, Order, Position — all serde-serializable
  indicators/        Pure stateless functions: ma, ema, rsi, atr, vwap, bollinger_bands
  feed/              WebSocket manager — up to 1,000 symbol subscriptions, auto-reconnect, tick fan-out
  strategy/
    dsl/             Compiler pipeline: lexer → parser → ast → validator
    runtime/         StrategyEngine — candle-by-candle evaluation loop
                     EvalContext, CrossDetector, TriggerStateMap, IndicatorProvider, BoundedWindowProvider
    execution/       ExecutionTarget trait; PaperBroker, DhanBroker; order_builder
    logging/         StrategyLogger, LogEntry, LogEntryKind
    analytics.rs     Backtest result metrics
    tests/           Integration tests
  commands/          Tauri IPC commands (data.rs, strategy.rs) — bridge Rust → React
  bin/behavioral_backtest.rs   CLI backtest runner (uses commands::strategy::run_backtest_internal)
src-tauri/
  src/main.rs        Tauri app entrypoint — registers commands, loads .env, sets up DataState
src/                 React frontend (TypeScript, Vite, React 19, lightweight-charts)
  components/        UI primitives: Button, Sidebar, TitleBar, RuleRow, IndicatorPicker, etc.
  screens/           Top-level screens: Builder, Strategies, StrategyCoder, StrategyUploader, Settings
  hooks/             useBacktest, useDslSync, useStrategyBuilder — bridge Tauri IPC + UI state
  types/             Shared TypeScript types matching Rust models
```

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
indicator      = ema | ma | rsi | atr | vwap | bb_upper | bb_lower | bb_mid
price_field    = close | open | high | low | volume
action         = BUY <int> | SELL <int> | SELL ALL
```

Blank lines and `# comments` allowed anywhere. Examples are in the `strategies/` directory and the README.

## Tauri IPC

`src-tauri/src/main.rs` is a thin shell. The actual command implementations live in `src/commands/`. Each is a free function taking `&DataState` (or similar) — the Tauri wrapper just unwraps `State<'_, AppState>` and forwards. The CLI binary `behavioral_backtest` calls `run_backtest_internal` directly to avoid spawning a Tauri runtime.

**Env requirement:** Tauri app requires `DHAN_ACCESS_TOKEN` in `.env` (see `.env.example`). The CLI also loads `.env` via `dotenvy::dotenv()`.

## Frontend ↔ Rust Contract

- Rust models serialize to camelCase JSON. TypeScript mirrors in `src/types/` (e.g. `types/strategy.ts`, `types/backtest.ts`, `types/tauri.ts`).
- IPC calls go through `@tauri-apps/api` `invoke()` — see `src/hooks/useBacktest.ts` for the pattern.
- Charts use `lightweight-charts` (TradingView). Candle data is piped Rust → React via Tauri IPC commands.

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
