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

## Critical Architecture Invariants

Don't break these — they are non-obvious properties the codebase is built around.

**1. Determinism.** Same strategy + same candles = identical output. Use `BTreeMap` over `HashMap` anywhere iteration order could affect evaluation. No randomness on eval paths. The engine uses `BTreeMap`-backed registries. (Guarded by a dedicated test.)

**2. Trigger state machine.** A bare `WHEN x > y / BUY 1` would fire every candle. `TriggerStateMap` fires rules only on `false → true` transitions. `CrossDetector` stores `(fast_prev, slow_prev)` and fires on the exact transition candle. Critically: the cross update pass runs **after** all rules are evaluated for the cycle — not inside the rule loop — so every rule sees consistent previous-candle state within a single cycle. See `src/strategy/runtime/cross.rs` + `trigger_state.rs`, called from `engine.rs::on_candle`.

**3. One execution engine.** Backtests, paper, and live all run through `StrategyEngine` + `ExecutionTarget`. The engine never knows which broker is behind the target. `PaperBroker` and `DhanBroker` are interchangeable implementations. (`src/strategy/execution/`.)

**4. Indicator windows are bounded.** `BoundedWindowProvider` keeps indicators O(N) — 184,863 NIFTY 1-min candles complete in ~3.5s (~52k candles/sec). Don't refactor into naive full-history recomputation. (`src/strategy/runtime/incremental_provider.rs`.)

**5. DSL grammar is intentionally small.** Rules only — no variables, no loops. Keywords case-insensitive. Indicator periods and quantities must be positive integers. `position_expr` and `time_window` parse but are not yet evaluated at runtime — see `src/strategy/dsl/ast.rs` and the `NotYetImplemented` branch in the evaluator.

**6. Wire-format boundary.** Rust types crossing the IPC boundary serialize to camelCase JSON (`#[serde(rename_all = "camelCase")]`). Internal `BacktestResult` / `PaperTrade` keep snake_case Rust field names; the conversion happens in `commands::strategy::BacktestResultWire` and `PaperTradeWire`. If you add a field the UI consumes, add it to the wire struct AND mirror it on the TS side — don't put extra UI-relevant fields directly on `BacktestResult` and rely on Rust serde to rename.

## Test Layout

- Per-module unit tests live alongside source as `#[cfg(test)] mod tests` blocks.
- Strategy integration tests live in `src/strategy/tests/` (notably `backtest_integration.rs`).
- A dedicated determinism test guards the same-input-same-output property.
- No frontend test suite is wired up — verify UI changes manually via `npm run tauri dev`.

## Env Requirement

Tauri app requires `DHAN_ACCESS_TOKEN` in `.env` (see `.env.example`). The CLI loads `.env` via `dotenvy::dotenv()`. When the token is missing or Dhan returns no candles, `run_backtest_dsl` falls back to the bundled sample CSV and emits an `eprintln!` warning.