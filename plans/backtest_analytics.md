## Codex Prompt

```
# AlgoMLN — Phase 2.95: RuleSkipped log variant + Backtest Result Analyser

Two things in one task. Do them in order. Do not mix the changes.

---

## Part 1: RuleSkipped log variant (small, do this first)

### Problem
When SELL ALL fires but no position exists, `order_builder::build_order`
returns `OrderBuildError::NoPosition`. The engine currently logs this as
`LogEntryKind::OrderFailed`, which prints in the CLI as an error. This is
not an error — it is expected behaviour. The sell condition was true but
there was nothing to sell. It should be silent in normal output.

### Change 1 — logging/log.rs

Add a new variant to `LogEntryKind`:

```rust
RuleSkipped {
    rule_id: String,
    reason: RuleSkipReason,
},
```

Add a new enum in the same file:

```rust
#[derive(Debug, Clone, Serialize)]
pub enum RuleSkipReason {
    NoPosition,         // SELL ALL fired but no open position exists
    // future: MaxOrdersReached, OutsideTimeWindow, etc.
}
```

Do not remove `OrderFailed`. That variant remains for genuine execution
failures (InsufficientFunds, broker errors). Only `NoPosition` moves to
`RuleSkipped`.

### Change 2 — runtime/engine.rs

In the `on_candle` execution path, after `build_order` returns an error:

Currently:
```rust
// pseudocode of current behaviour
Err(OrderBuildError::NoPosition) => log OrderFailed { error: "no position" }
Err(OrderBuildError::ZeroQuantity) => log OrderFailed { error: "zero quantity" }
```

Change to:
```rust
Err(OrderBuildError::NoPosition) => {
    self.logger.log(LogEntryKind::RuleSkipped {
        rule_id: rule.id.clone(),
        reason: RuleSkipReason::NoPosition,
    }, ctx.current.timestamp);
    continue;
}
Err(OrderBuildError::ZeroQuantity) => {
    // ZeroQuantity is a real bug (validator should have caught it)
    // keep as OrderFailed so it stays visible
    self.logger.log(LogEntryKind::OrderFailed {
        rule_id: rule.id.clone(),
        error: "zero quantity — validator missed this".to_string(),
    }, ctx.current.timestamp);
    continue;
}
```

### Change 3 — bin/behavioral_backtest.rs

In the CLI log printer, update the filter that decides what to print.
`RuleSkipped` entries must NOT be printed. They are informational only.

If there is a summary section that counts skipped rules, add:
```
Skipped (no position)   <count>
```
to the output table. Count = number of `RuleSkipped { reason: NoPosition }`
entries across all log entries.

### Tests

No new unit tests needed for this change — the existing engine integration
tests cover the execution path. Confirm `cargo test` still passes at 105.

---

## Part 2: Backtest Result Analyser

### New file: src-tauri/src/strategy/analytics.rs

Add `pub mod analytics;` to `src-tauri/src/strategy/mod.rs`.

### BacktestSummary struct

```rust
use serde::{Serialize, Deserialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BacktestSummary {
    // Capital
    pub initial_cash: f64,
    pub final_cash: f64,
    pub total_return_pct: f64,       // (final - initial) / initial * 100

    // Trades
    pub total_trades: usize,         // buys + sells
    pub buy_count: usize,
    pub sell_count: usize,
    pub closed_trades: usize,        // sells with pnl.is_some()

    // Win/Loss
    pub winning_trades: usize,       // closed trades where pnl > 0
    pub losing_trades: usize,        // closed trades where pnl < 0
    pub breakeven_trades: usize,     // closed trades where pnl == 0.0
    pub win_rate_pct: f64,           // winning / closed * 100. 0.0 if no closed trades.

    // PnL
    pub total_realized_pnl: f64,
    pub gross_profit: f64,           // sum of positive pnls
    pub gross_loss: f64,             // sum of negative pnls (will be <= 0)
    pub profit_factor: f64,          // gross_profit / |gross_loss|. 0.0 if no losses.
    pub avg_win: f64,                // 0.0 if no wins
    pub avg_loss: f64,               // 0.0 if no losses (negative when non-zero)
    pub largest_win: f64,
    pub largest_loss: f64,           // most negative pnl seen. 0.0 if no losses.
    pub expectancy: f64,             // (win_rate * avg_win) + ((1 - win_rate) * avg_loss)

    // Drawdown
    /// Peak-to-trough on the running PnL curve. Always >= 0.
    pub max_drawdown: f64,
    pub max_drawdown_pct: f64,       // max_drawdown / initial_cash * 100

    // Consistency
    pub max_consecutive_wins: usize,
    pub max_consecutive_losses: usize,

    // Run info
    pub total_candles_processed: usize,
    pub candles_per_trade: f64,      // total_candles / closed_trades. 0.0 if no closed trades.
    pub skipped_no_position: usize,  // count of RuleSkipped::NoPosition log entries
}
```

### BacktestAnalyser

```rust
pub struct BacktestAnalyser;

impl BacktestAnalyser {
    pub fn analyse(
        trade_history: &[PaperTrade],
        initial_cash: f64,
        final_cash: f64,
        total_candles_processed: usize,
        logs: &[LogEntry],           // needed for skipped_no_position count
    ) -> BacktestSummary
}
```

### Computation rules

**Capital:**
`total_return_pct = (final_cash - initial_cash) / initial_cash * 100.0`

**Trades:**
- `buy_count` = entries where `side == OrderSide::Buy`
- `sell_count` = entries where `side == OrderSide::Sell` (any variant)
- `total_trades = buy_count + sell_count`
- `closed_trades` = entries where `pnl.is_some()`

**Win/Loss:** over closed trades only:
- `winning_trades`: pnl > 0.0
- `losing_trades`: pnl < 0.0
- `breakeven_trades`: pnl == 0.0
- `win_rate_pct = if closed_trades == 0 { 0.0 } else { winning as f64 / closed as f64 * 100.0 }`

**PnL:**
- `total_realized_pnl` = sum of all `pnl.unwrap_or(0.0)`
- `gross_profit` = sum of pnl > 0.0
- `gross_loss` = sum of pnl < 0.0
- `profit_factor = if gross_loss == 0.0 { 0.0 } else { gross_profit / gross_loss.abs() }`
- `avg_win = if winning_trades == 0 { 0.0 } else { gross_profit / winning_trades as f64 }`
- `avg_loss = if losing_trades == 0 { 0.0 } else { gross_loss / losing_trades as f64 }`
- `largest_win` = max pnl value, 0.0 if no closed trades
- `largest_loss` = min pnl value, 0.0 if no closed trades
- `expectancy`:
  ```
  let wr = win_rate_pct / 100.0;
  if closed_trades == 0 { 0.0 }
  else { (wr * avg_win) + ((1.0 - wr) * avg_loss) }
  ```

**Drawdown** (over the running PnL series of closed trades):
```
peak = 0.0
max_drawdown = 0.0
running_pnl = 0.0
for each closed trade in order:
    running_pnl += pnl
    if running_pnl > peak { peak = running_pnl }
    let dd = peak - running_pnl
    if dd > max_drawdown { max_drawdown = dd }
max_drawdown_pct = max_drawdown / initial_cash * 100.0
```

**Consecutive streaks** (over closed trades in order):
```
current_wins = 0, current_losses = 0
max_consecutive_wins = 0, max_consecutive_losses = 0
for each closed trade:
    if pnl > 0: current_wins += 1; current_losses = 0
    elif pnl < 0: current_losses += 1; current_wins = 0
    else: current_wins = 0; current_losses = 0
    update maxes
```

**Skipped:** count log entries where
`matches!(entry.kind, LogEntryKind::RuleSkipped { reason: RuleSkipReason::NoPosition, .. })`

**candles_per_trade:**
`if closed_trades == 0 { 0.0 } else { total_candles_processed as f64 / closed_trades as f64 }`

### Wire into BacktestResult

In `src-tauri/src/commands/strategy.rs`, update `BacktestResult`:

```rust
#[derive(Debug, Serialize)]
pub struct BacktestResult {
    pub trade_history: Vec<PaperTrade>,
    pub final_cash: f64,
    pub initial_cash: f64,          // add if missing
    pub total_realized_pnl: f64,
    pub total_candles_processed: usize,
    pub logs: Vec<LogEntry>,
    pub summary: BacktestSummary,   // add this
}
```

At the end of `run_backtest_internal`, after the candle loop:
```rust
let state = broker.get_state();
let summary = BacktestAnalyser::analyse(
    &state.trade_history,
    initial_cash,
    state.cash,
    total_candles_processed,
    &all_logs,
);
Ok(BacktestResult {
    trade_history: state.trade_history,
    final_cash: state.cash,
    initial_cash,
    total_realized_pnl: state.total_realized_pnl,
    total_candles_processed,
    logs: all_logs,
    summary,
})
```

### CLI output

In `bin/behavioral_backtest.rs`, replace raw trade printing with this format.
Use `format!` and string padding only — no external table crates.

```
════════════════════════════════════════════════
 BACKTEST — {strategy_name}
 Symbol: {symbol}   Candles: {n}
════════════════════════════════════════════════

 CAPITAL
   Initial       ₹ {initial_cash:>15.2}
   Final         ₹ {final_cash:>15.2}
   Return          {return_pct:>+14.2}%

 TRADES
   Total         {total_trades:>16}   ({buy_count} buys, {sell_count} sells)
   Closed        {closed_trades:>16}
   Win rate      {win_rate_pct:>15.2}%   ({W}W / {L}L / {B}B)

 PnL
   Realized      ₹ {total_realized_pnl:>+14.2}
   Gross profit  ₹ {gross_profit:>15.2}
   Gross loss    ₹ {gross_loss:>15.2}
   Profit factor   {profit_factor:>14.2}
   Avg win       ₹ {avg_win:>+14.2}
   Avg loss      ₹ {avg_loss:>+14.2}
   Largest win   ₹ {largest_win:>+14.2}
   Largest loss  ₹ {largest_loss:>+14.2}
   Expectancy    ₹ {expectancy:>+14.2}

 RISK
   Max drawdown  ₹ {max_drawdown:>14.2}  ({max_drawdown_pct:.3}%)
   Max consec W    {max_consecutive_wins:>14}
   Max consec L    {max_consecutive_losses:>14}

 THROUGHPUT
   Candles/trade   {candles_per_trade:>14.1}
   Skipped (no pos){skipped_no_position:>13}

════════════════════════════════════════════════
```

If `closed_trades == 0`, print after the TRADES section:
```
 ⚠  No closed trades — strategy never completed a buy→sell cycle.
```
and skip PnL and RISK sections entirely.

Use `\u{2550}` (═) for borders. Positive PnL values get `+` prefix via Rust's
`{:+}` format. Currency is ₹. Numbers are right-aligned.

### Tests

Add `#[cfg(test)]` at the bottom of `analytics.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::execution::paper::PaperTrade;
    use crate::broker::models::order::OrderSide;
    use chrono::Utc;

    fn buy(price: f64) -> PaperTrade {
        PaperTrade {
            id: "x".to_string(),
            timestamp: Utc::now(),
            symbol: "T".to_string(),
            side: OrderSide::Buy,
            quantity: 1,
            price,
            rule_id: "rule_0".to_string(),
            pnl: None,
        }
    }

    fn sell(price: f64, pnl: f64) -> PaperTrade {
        PaperTrade {
            id: "x".to_string(),
            timestamp: Utc::now(),
            symbol: "T".to_string(),
            side: OrderSide::Sell,
            quantity: 1,
            price,
            rule_id: "rule_0".to_string(),
            pnl: Some(pnl),
        }
    }

    fn analyse(trades: &[PaperTrade], initial: f64, final_cash: f64, candles: usize) -> BacktestSummary {
        BacktestAnalyser::analyse(trades, initial, final_cash, candles, &[])
    }

    #[test]
    fn empty_history() {
        let s = analyse(&[], 100_000.0, 100_000.0, 100);
        assert_eq!(s.total_trades, 0);
        assert_eq!(s.closed_trades, 0);
        assert_eq!(s.win_rate_pct, 0.0);
        assert_eq!(s.max_drawdown, 0.0);
        assert_eq!(s.total_return_pct, 0.0);
    }

    #[test]
    fn single_win() {
        let s = analyse(&[buy(100.0), sell(120.0, 20.0)], 100_000.0, 100_020.0, 50);
        assert_eq!(s.closed_trades, 1);
        assert_eq!(s.winning_trades, 1);
        assert_eq!(s.win_rate_pct, 100.0);
        assert_eq!(s.gross_profit, 20.0);
        assert_eq!(s.max_drawdown, 0.0);
    }

    #[test]
    fn single_loss() {
        let s = analyse(&[buy(100.0), sell(80.0, -20.0)], 100_000.0, 99_980.0, 50);
        assert_eq!(s.losing_trades, 1);
        assert_eq!(s.win_rate_pct, 0.0);
        assert_eq!(s.gross_loss, -20.0);
        assert_eq!(s.largest_loss, -20.0);
        // peak=0, after trade running_pnl=-20, drawdown=20
        assert_eq!(s.max_drawdown, 20.0);
    }

    #[test]
    fn drawdown_win_then_loss() {
        // pnl: +100 then -60 → running: 100, 40 → peak=100, dd=60
        let trades = vec![
            buy(100.0), sell(200.0, 100.0),
            buy(200.0), sell(140.0, -60.0),
        ];
        let s = analyse(&trades, 100_000.0, 100_040.0, 100);
        assert_eq!(s.max_drawdown, 60.0);
        assert_eq!(s.total_realized_pnl, 40.0);
    }

    #[test]
    fn profit_factor() {
        // gross_profit=80, gross_loss=-30 → pf = 80/30
        let trades = vec![
            buy(100.0), sell(150.0, 50.0),
            buy(100.0), sell(130.0, 30.0),
            buy(100.0), sell(80.0, -20.0),
            buy(100.0), sell(90.0, -10.0),
        ];
        let s = analyse(&trades, 100_000.0, 100_050.0, 200);
        assert!((s.profit_factor - 80.0 / 30.0).abs() < 0.001);
    }

    #[test]
    fn consecutive_streaks() {
        // W W W L L W L W W → max_W=3 max_L=2
        let pnls = [10.0, 10.0, 10.0, -5.0, -5.0, 10.0, -5.0, 10.0, 10.0];
        let mut trades = Vec::new();
        for &p in &pnls {
            trades.push(buy(100.0));
            trades.push(sell(if p > 0.0 { 110.0 } else { 95.0 }, p));
        }
        let s = analyse(&trades, 100_000.0, 100_045.0, 400);
        assert_eq!(s.max_consecutive_wins, 3);
        assert_eq!(s.max_consecutive_losses, 2);
    }

    #[test]
    fn breakeven_included_in_closed_not_win_or_loss() {
        let trades = vec![
            buy(100.0), sell(110.0, 10.0),
            buy(100.0), sell(100.0, 0.0),
            buy(100.0), sell(90.0, -10.0),
        ];
        let s = analyse(&trades, 100_000.0, 100_000.0, 150);
        assert_eq!(s.closed_trades, 3);
        assert_eq!(s.winning_trades, 1);
        assert_eq!(s.breakeven_trades, 1);
        assert_eq!(s.losing_trades, 1);
        // win_rate = 1/3 * 100
        assert!((s.win_rate_pct - 33.333).abs() < 0.01);
    }

    #[test]
    fn only_buys_no_closed() {
        let s = analyse(&[buy(100.0), buy(200.0)], 100_000.0, 70_000.0, 50);
        assert_eq!(s.buy_count, 2);
        assert_eq!(s.closed_trades, 0);
        assert_eq!(s.max_drawdown, 0.0);
        assert_eq!(s.candles_per_trade, 0.0);
    }

    #[test]
    fn total_return_pct() {
        let s = analyse(&[], 100_000.0, 110_000.0, 0);
        assert!((s.total_return_pct - 10.0).abs() < 0.001);
    }

    #[test]
    fn candles_per_trade() {
        let trades: Vec<PaperTrade> = (0..4).flat_map(|_|
            vec![buy(100.0), sell(110.0, 10.0)]
        ).collect();
        let s = analyse(&trades, 100_000.0, 100_040.0, 100);
        assert_eq!(s.candles_per_trade, 25.0);
    }
}
```

---

## Final verification

```bash
cargo test
```

Expected: 105 existing + ~10 new analytics tests = ~115 passing, 0 failed.

The CLI output for any existing `.algomln` backtest must show the summary
table and zero `NoPosition` lines in the output.

## What NOT to do

- Do not remove `OrderFailed` from `LogEntryKind`
- Do not modify any indicator functions
- Do not add external crates
- Do not change the engine evaluation logic
- Do not touch `DhanClient` or `BrokerClient`
```