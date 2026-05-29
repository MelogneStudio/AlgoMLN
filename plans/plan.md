# Dhan Client — Full App Plan

## What It Is
A desktop trading client built on Dhan API (Upstox later).
Polished, fast, offline, no bloat.
Works out of the box for normal traders. Goes deep for algo people.

---

## Core Principles
- Fast cold start
- No account required
- Nothing phones home
- Depth is always there, never forced
- Paper trade before live trade, always

---

## Tech Stack
```
Tauri (Rust)
  └── Rust backend (all logic, data, indicators, algo engine)
  └── Tauri commands (no localhost, direct IPC)
  └── React frontend (webview)
```

---

## Broker Abstraction (Do This First)
```rust
trait BrokerClient {
    fn get_ohlcv(symbol, timeframe, from, to) -> Vec<Candle>
    fn subscribe_ticks(symbols: Vec<Symbol>) -> Stream<Tick>
    fn get_quote(symbol) -> Quote
    fn place_order(order: Order) -> OrderResult
    fn get_positions() -> Vec<Position>
    fn get_portfolio() -> Portfolio
}
```
- `DhanClient` implements this now
- `UpstoxClient` slots in later — nothing else changes

---

## Phase 1 — Data Layer
> Foundation. Everything depends on this being solid.

- Broker abstraction trait
- `DhanClient` implementation
- Data models: `Candle`, `Tick`, `Quote`, `Order`, `Position`
- WebSocket manager (up to 1000 subscriptions, auto-reconnect)
- Tick fan-out to internal subscribers
- OHLCV fetch + historical data
- Tauri commands exposing data to React

---

## Phase 2 — Indicator Engine
> Built on clean candle data. Pure functions.

- MA, EMA, BB, ATR, RSI, VWAP (core set)
- Each indicator: `Vec<Candle> -> Vec<f64>`
- Stateless, fast, easily testable
- Extendable — easy to add more later

---

## Phase 3 — Charts + Core UI
> First thing the user actually sees.

- Interactive charts via lightweight-charts (TradingView)
- Candle + indicator data piped from Rust → React
- Indicator toggle panel
- S/R overlay (already built)
- Timeframe selector
- Symbol search + switcher

---

## Phase 4 — Built-in Tools
> Pre-built stuff traders actually use.

- Option chain viewer
- OI (Open Interest) analysis
- Payoff graph
- Screener (later)

---

## Phase 5 — Visual Algo Builder
> Normal person path. No code needed.

- Drag-drop flow builder in React
- Block types: Trigger, Condition, Action, Wait
- Plain english labels ("Price crosses above", "Buy X shares")
- Compiles down to algo engine internally
- Covers 80% of use cases

---

## Phase 6 — Scripting Layer
> Power user path.

- Embedded Python or Rhai (Rust-native scripting, evaluate later)
- Clean DSL on top:
  ```python
  when(cross(MA(20), MA(50)))
    .buy(quantity=10)
    .confirm()
  ```
- Sandboxed execution (restricted imports, timeout)
- Full access to indicator engine + broker abstraction

---

## Phase 7 — Algo Execution
> The trading engine. Handle with care.

- Paper trading mode always available, always default
- Live trading requires:
  1. Algo validated in paper mode
  2. Two explicit hard confirmation buttons
  3. User-set risk limits (max loss, max orders)
- Immutable trade log — every order logged with timestamp + algo state
- "This is not financial advice" shown prominently
- Risk acknowledgment on first live trade

---

## What Gets Built When
```
Phase 1  →  Data layer + broker abstraction
Phase 2  →  Indicators
Phase 3  →  Charts + UI (first shippable version)
Phase 4  →  Tools
Phase 5  →  Visual algo builder
Phase 6  →  Scripting layer
Phase 7  →  Live execution
```

Don't touch Phase 2 until Phase 1 is solid.
Don't touch Phase 7 until paper trading is well tested.