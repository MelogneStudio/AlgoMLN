# AlgoMLN Strategy DSL Reference

The AlgoMLN Domain Specific Language (DSL) allows you to define trading strategies using a concise, human-readable syntax. Strategies are evaluated candle-by-candle deterministically.

## Strategy Structure

A strategy consists of an optional header, optional global declarations, and one or more rules.

```
[TRADE_IN <symbols/index>]
[STOP_LOSS <value>%]
[TAKE_PROFIT <value>%]
[RISK <config>]
WHEN <condition>
<action>
```

## 1. Header: `TRADE_IN`

The `TRADE_IN` header specifies which assets the strategy should be applied to.

- **Symbols**: A comma-separated list of symbols.
  - Example: `TRADE_IN RELIANCE, INFY, TCS`
- **Index Aliases**: A single index alias.
  - Example: `TRADE_IN NIFTY_BANK`

*Note: You cannot mix symbols and index aliases in a single `TRADE_IN` statement.*

## 2. Global Declarations

These declarations set strategy-level safety nets and risk controls. They can be placed at the top or interleaved between rules.

### Safety Nets
- **`STOP_LOSS <number>%`**: Automatically closes the position if the loss reaches the specified percentage.
  - Example: `STOP_LOSS 2%`
- **`TAKE_PROFIT <number>%`**: Automatically closes the position if the profit reaches the specified percentage.
  - Example: `TAKE_PROFIT 5%`

### Risk Controls
Defined using the `RISK` keyword:
- **`RISK MAX_DAILY_LOSS <number>%`**: Stops trading for the day if the cumulative realized loss exceeds this percentage of initial capital.
  - Example: `RISK MAX_DAILY_LOSS 5%`
- **`RISK MAX_POSITIONS <integer>`**: Limits the number of concurrent open positions.
  - Example: `RISK MAX_POSITIONS 3`
- **`RISK MAX_ORDERS <integer>`**: Limits the total number of entry orders per session.
  - Example: `RISK MAX_ORDERS 20`

## 3. Rules

Rules are the core logic of the strategy. Each rule follows the format:
`WHEN <condition>`
`<action>`

### Conditions

Conditions determine when an action should be triggered.

#### Expressions
Expressions are the building blocks of conditions.
- **Literals**: `10`, `10.5`
- **Price Fields**: `close`, `open`, `high`, `low`, `volume`, `prev_close`, `prev_open`, `prev_high`, `prev_low`.
- **Indicators**:
  - `ema(period)` - Exponential Moving Average
  - `ma(period)` - Simple Moving Average
  - `rsi(period)` - Relative Strength Index
  - `rel_vol(period)` - Relative Volume
  - `atr(period)` - Average True Range
  - `vwap(period)` - Volume Weighted Average Price
  - `bb_upper(period)`, `bb_lower(period)`, `bb_mid(period)` - Bollinger Bands

#### Comparison Operators
Used to compare two expressions: `<`, `>`, `<=`, `>=`, `==`, `!=`.
- Example: `WHEN rsi(14) < 30`
- Example: `WHEN close > bb_upper(20)`

#### Crosses
Detects when one expression crosses another.
- `cross_above(fast, slow)`: True on the candle where `fast` becomes greater than `slow`.
- `cross_below(fast, slow)`: True on the candle where `fast` becomes less than `slow`.
- Example: `WHEN cross_above(ema(20), ema(50))`

#### Boolean Logic & State
- **`AND`, `OR`, `NOT (...)`**: Combine multiple conditions.
- **`in_position()`**: True if there is currently an open position.
- **`between(HH:MM, HH:MM)`**: True if the current candle time is within the specified window.
- Example: `WHEN cross_above(ema(20), ema(50)) AND NOT (in_position())`
- Example: `WHEN between(09:15, 11:00) AND close > prev_high`

### Actions

Actions define what the strategy does when a condition is met.

#### Buying
- `BUY <quantity>`
  - **Fixed**: `BUY 10` (10 shares)
  - **Percent Capital**: `BUY 10%` (10% of available capital)
  - **Value Based**: `BUY 5000 WORTH` (shares worth 5000 rupees)

#### Selling
- `SELL <quantity>` (Same quantity options as BUY)
- `SELL ALL`: Closes the entire open position regardless of quantity.

## Examples

### RSI Mean Reversion
```
TRADE_IN NIFTY
STOP_LOSS 2%
TAKE_PROFIT 5%

WHEN rsi(14) < 30
BUY 10%

WHEN rsi(14) > 70
SELL ALL
```

### EMA Crossover with Risk Controls
```
TRADE_IN RELIANCE, INFY
RISK MAX_POSITIONS 2
RISK MAX_ORDERS 10

WHEN cross_above(ema(20), ema(50)) AND NOT (in_position())
BUY 5000 WORTH

WHEN cross_below(ema(20), ema(50))
SELL ALL
```
