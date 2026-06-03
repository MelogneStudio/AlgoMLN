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
88 tests passing · 0 failed · 1 ignored
```

### ✅ Phase 1 — Data Layer
- Broker abstraction trait (`BrokerClient`)
- `DhanClient` implementation
- Data models: `Candle`, `Tick`, `Quote`, `Order`, `Position`
- WebSocket manager — up to 1,000 symbol subscriptions, auto-reconnect
- Tick fan-out to internal subscribers
- Historical OHLCV fetch
- Tauri IPC commands exposing data to React

### ✅ Phase 2 — Indicator Engine
Pure Rust functions. Stateless. `fn indicator(candles: &[Candle], period: usize) -> Vec<f64>`.

| Indicator | Function |
|---|---|
| Simple Moving Average | `ma` |
| Exponential Moving Average | `ema` |
| Relative Strength Index | `rsi` |
| Average True Range | `atr` |
| Volume Weighted Average Price | `vwap` |
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
indicator      = "ema" | "ma" | "rsi" | "atr" | "vwap"
               | "bb_upper" | "bb_lower" | "bb_mid"

price_field    = "close" | "open" | "high" | "low" | "volume"

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

```
src-tauri/src/
  broker/
    mod.rs              BrokerClient trait
    dhan/               DhanClient implementation
  models/               Candle, Tick, Quote, Order, Position
  indicators/           Pure indicator functions
  strategy/
    dsl/
      lexer.rs          Lexer + Token types
      parser.rs         Recursive descent parser
      ast.rs            All AST node types (serializable)
      validator.rs      AstValidator — collects all errors
    runtime/
      engine.rs         StrategyEngine — main evaluation loop
      context.rs        EvalContext — per-candle borrowed view
      cross.rs          CrossDetector
      trigger_state.rs  TriggerStateMap
      indicator_provider.rs  IndicatorProvider trait + BoundedWindowProvider
    execution/
      target.rs         ExecutionTarget trait
      paper.rs          PaperBroker
      order_builder.rs  Builds Order from ActionNode
    logging/
      log.rs            StrategyLog, LogEntry, LogEntryKind
    mod.rs              StrategyRegistry
  commands/
    strategy.rs         Tauri IPC commands
  bin/
    behavioral_backtest.rs  CLI backtest runner
```

---

## Roadmap

### Phase 3 — Charts & Core UI
- Lightweight Charts (TradingView) integration
- Candle data piped Rust → React via Tauri IPC
- Indicator overlays with toggle panel
- Support/Resistance overlay
- Timeframe selector
- Symbol search and switcher

### Phase 4 — Trading Tools
- Option chain viewer
- Open Interest analysis
- Payoff diagrams
- Screener

### Phase 5 — Visual Strategy Builder
```
Drag-and-drop blocks → Generated DSL → AST → Strategy Engine
```
Same runtime as text strategies. No separate execution path.

### Phase 6 — Advanced Strategy Features
- Position sizing rules
- Stop loss / take profit
- Risk controls
- Multi-symbol strategies

### Phase 7 — Live Trading
- Live broker execution via `ExecutionTarget`
- Two hard confirmation steps required
- User-defined risk limits (max loss, max orders)
- Immutable trade log
- Risk acknowledgment on first live trade

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