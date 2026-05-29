# Data Layer — Detailed Plan

## Goal
Build a clean, broker-agnostic data foundation that everything else sits on top of.
Get this wrong and everything built on it is sand.

---

## Folder Structure
```
src/
  broker/
    mod.rs          ← BrokerClient trait + shared types
    dhan/
      mod.rs        ← DhanClient implementation
      auth.rs       ← API key handling
      rest.rs       ← REST calls (OHLCV, quotes, orders)
      websocket.rs  ← Live tick feed
      models.rs     ← Dhan-specific response shapes (internal)
  models/
    candle.rs       ← Candle struct
    tick.rs         ← Tick struct
    quote.rs        ← Quote struct
    order.rs        ← Order, OrderResult
    position.rs     ← Position, Portfolio
  feed/
    manager.rs      ← WebSocket manager (subscription, reconnect, fan-out)
  commands/
    data.rs         ← Tauri commands exposed to React
```

---

## Core Data Models

```rust
// candle.rs
pub struct Candle {
    pub timestamp: i64,   // Unix ms
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

// tick.rs
pub struct Tick {
    pub symbol: String,
    pub ltp: f64,         // Last traded price
    pub volume: u64,
    pub timestamp: i64,
}

// quote.rs
pub struct Quote {
    pub symbol: String,
    pub ltp: f64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub bid: f64,
    pub ask: f64,
    pub volume: u64,
}

// order.rs
pub struct Order {
    pub symbol: String,
    pub side: OrderSide,  // Buy | Sell
    pub quantity: u32,
    pub order_type: OrderType, // Market | Limit | SL
    pub price: Option<f64>,
}

pub struct OrderResult {
    pub order_id: String,
    pub status: OrderStatus,
    pub timestamp: i64,
}
```

---

## Broker Trait

```rust
// broker/mod.rs
#[async_trait]
pub trait BrokerClient: Send + Sync {
    async fn get_ohlcv(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
    ) -> Result<Vec<Candle>>;

    async fn get_quote(&self, symbol: &str) -> Result<Quote>;

    async fn place_order(&self, order: Order) -> Result<OrderResult>;

    async fn get_positions(&self) -> Result<Vec<Position>>;

    async fn get_portfolio(&self) -> Result<Portfolio>;

    // WebSocket feed handled separately by FeedManager
}

pub enum Timeframe {
    M1, M5, M15, M30,
    H1, H4,
    D1, W1,
}
```

---

## WebSocket Feed Manager

```rust
// feed/manager.rs

// Responsibilities:
// - Maintain single WS connection to Dhan
// - Handle subscribe/unsubscribe for up to 1000 symbols
// - Auto-reconnect on drop (exponential backoff)
// - Fan out ticks to multiple internal receivers via channels

pub struct FeedManager {
    subscriptions: HashSet<String>,
    sender: broadcast::Sender<Tick>,
    // internal WS handle
}

impl FeedManager {
    pub async fn subscribe(&mut self, symbols: Vec<String>)
    pub async fn unsubscribe(&mut self, symbols: Vec<String>)
    pub fn subscribe_ticks(&self) -> broadcast::Receiver<Tick>
    // internal reconnect loop
}
```

---

## Tauri Commands (React ↔ Rust)

```rust
// commands/data.rs

#[tauri::command]
async fn get_ohlcv(
    symbol: String,
    timeframe: String,
    from: i64,
    to: i64,
) -> Result<Vec<Candle>, String>

#[tauri::command]
async fn get_quote(symbol: String) -> Result<Quote, String>

#[tauri::command]
async fn subscribe_ticks(symbols: Vec<String>) -> Result<(), String>

// Ticks pushed to React via Tauri events (not commands)
// broker emits: app.emit_all("tick", tick_payload)
```

---

## What React Receives

```ts
// Candle shape for lightweight-charts
{
  time: number,       // Unix timestamp (seconds)
  open: number,
  high: number,
  low: number,
  close: number,
  volume: number,
}

// Tick event payload
{
  symbol: string,
  ltp: number,
  volume: number,
  timestamp: number,
}
```

---

## Build Order

1. Define all models (`Candle`, `Tick`, `Quote`, `Order`, `Position`)
2. Define `BrokerClient` trait
3. Implement `DhanClient` REST (OHLCV + quotes first)
4. Wire up Tauri command for `get_ohlcv`
5. Get candles rendering in React with lightweight-charts
6. Implement `FeedManager` (WebSocket)
7. Wire tick events to React
8. Implement `place_order` + `get_positions` (needed for Phase 7)

---

## Rules
- Phase 1 done = candles on screen, live ticks flowing, S/R overlay working
- No indicator logic in the data layer — pure data only
- No business logic in Tauri commands — they're thin wrappers
- All broker-specific types stay inside `broker/dhan/` — never leak to the rest of the app