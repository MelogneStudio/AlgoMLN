use serde::{Deserialize, Serialize};

use crate::models::OrderSide;
use crate::strategy::execution::PaperTrade;
use crate::strategy::logging::{LogEntry, LogEntryKind, RuleSkipReason};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestSummary {
    pub initial_cash: f64,
    pub final_cash: f64,
    pub total_return_pct: f64,
    pub total_trades: usize,
    pub buy_count: usize,
    pub sell_count: usize,
    pub closed_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,
    pub breakeven_trades: usize,
    pub win_rate_pct: f64,
    pub total_realized_pnl: f64,
    pub gross_profit: f64,
    pub gross_loss: f64,
    pub profit_factor: f64,
    pub avg_win: f64,
    pub avg_loss: f64,
    pub largest_win: f64,
    pub largest_loss: f64,
    pub expectancy: f64,
    pub max_drawdown: f64,
    pub max_drawdown_pct: f64,
    pub max_consecutive_wins: usize,
    pub max_consecutive_losses: usize,
    pub total_candles_processed: usize,
    pub candles_per_trade: f64,
    pub skipped_no_position: usize,
}

pub struct BacktestAnalyser;

impl BacktestAnalyser {
    pub fn analyse(
        trade_history: &[PaperTrade],
        initial_cash: f64,
        final_cash: f64,
        total_candles_processed: usize,
        logs: &[LogEntry],
    ) -> BacktestSummary {
        let buy_count = trade_history
            .iter()
            .filter(|trade| trade.side == OrderSide::Buy)
            .count();
        let sell_count = trade_history
            .iter()
            .filter(|trade| trade.side == OrderSide::Sell)
            .count();
        let closed_pnls = trade_history
            .iter()
            .filter_map(|trade| trade.pnl)
            .collect::<Vec<_>>();
        let closed_trades = closed_pnls.len();

        let winning_trades = closed_pnls.iter().filter(|&&pnl| pnl > 0.0).count();
        let losing_trades = closed_pnls.iter().filter(|&&pnl| pnl < 0.0).count();
        let breakeven_trades = closed_pnls.iter().filter(|&&pnl| pnl == 0.0).count();
        let win_rate_pct = if closed_trades == 0 {
            0.0
        } else {
            winning_trades as f64 / closed_trades as f64 * 100.0
        };

        let total_realized_pnl = closed_pnls.iter().sum::<f64>();
        let gross_profit = closed_pnls
            .iter()
            .copied()
            .filter(|pnl| *pnl > 0.0)
            .sum::<f64>();
        let gross_loss = closed_pnls
            .iter()
            .copied()
            .filter(|pnl| *pnl < 0.0)
            .sum::<f64>();
        let profit_factor = if gross_loss == 0.0 {
            0.0
        } else {
            gross_profit / gross_loss.abs()
        };
        let avg_win = if winning_trades == 0 {
            0.0
        } else {
            gross_profit / winning_trades as f64
        };
        let avg_loss = if losing_trades == 0 {
            0.0
        } else {
            gross_loss / losing_trades as f64
        };
        let largest_win = closed_pnls
            .iter()
            .copied()
            .filter(|pnl| *pnl > 0.0)
            .reduce(f64::max)
            .unwrap_or(0.0);
        let largest_loss = closed_pnls
            .iter()
            .copied()
            .filter(|pnl| *pnl < 0.0)
            .reduce(f64::min)
            .unwrap_or(0.0);
        let expectancy = if closed_trades == 0 {
            0.0
        } else {
            let win_rate = win_rate_pct / 100.0;
            (win_rate * avg_win) + ((1.0 - win_rate) * avg_loss)
        };

        let mut peak = 0.0;
        let mut running_pnl = 0.0;
        let mut max_drawdown = 0.0;
        for pnl in &closed_pnls {
            running_pnl += pnl;
            if running_pnl > peak {
                peak = running_pnl;
            }
            let drawdown = peak - running_pnl;
            if drawdown > max_drawdown {
                max_drawdown = drawdown;
            }
        }

        let mut current_wins = 0;
        let mut current_losses = 0;
        let mut max_consecutive_wins = 0;
        let mut max_consecutive_losses = 0;
        for pnl in &closed_pnls {
            if *pnl > 0.0 {
                current_wins += 1;
                current_losses = 0;
            } else if *pnl < 0.0 {
                current_losses += 1;
                current_wins = 0;
            } else {
                current_wins = 0;
                current_losses = 0;
            }
            max_consecutive_wins = max_consecutive_wins.max(current_wins);
            max_consecutive_losses = max_consecutive_losses.max(current_losses);
        }

        let skipped_no_position = logs
            .iter()
            .filter(|entry| {
                matches!(
                    &entry.kind,
                    LogEntryKind::RuleSkipped {
                        reason: RuleSkipReason::NoPosition,
                        ..
                    }
                )
            })
            .count();

        BacktestSummary {
            initial_cash,
            final_cash,
            total_return_pct: if initial_cash == 0.0 {
                0.0
            } else {
                (final_cash - initial_cash) / initial_cash * 100.0
            },
            total_trades: buy_count + sell_count,
            buy_count,
            sell_count,
            closed_trades,
            winning_trades,
            losing_trades,
            breakeven_trades,
            win_rate_pct,
            total_realized_pnl,
            gross_profit,
            gross_loss,
            profit_factor,
            avg_win,
            avg_loss,
            largest_win,
            largest_loss,
            expectancy,
            max_drawdown,
            max_drawdown_pct: if initial_cash == 0.0 {
                0.0
            } else {
                max_drawdown / initial_cash * 100.0
            },
            max_consecutive_wins,
            max_consecutive_losses,
            total_candles_processed,
            candles_per_trade: if closed_trades == 0 {
                0.0
            } else {
                total_candles_processed as f64 / closed_trades as f64
            },
            skipped_no_position,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn buy(price: f64) -> PaperTrade {
        PaperTrade {
            id: "x".to_string(),
            timestamp: Utc::now().timestamp_millis(),
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
            timestamp: Utc::now().timestamp_millis(),
            symbol: "T".to_string(),
            side: OrderSide::Sell,
            quantity: 1,
            price,
            rule_id: "rule_0".to_string(),
            pnl: Some(pnl),
        }
    }

    fn analyse(
        trades: &[PaperTrade],
        initial: f64,
        final_cash: f64,
        candles: usize,
    ) -> BacktestSummary {
        BacktestAnalyser::analyse(trades, initial, final_cash, candles, &[])
    }

    #[test]
    fn empty_history() {
        let summary = analyse(&[], 100_000.0, 100_000.0, 100);
        assert_eq!(summary.total_trades, 0);
        assert_eq!(summary.closed_trades, 0);
        assert_eq!(summary.win_rate_pct, 0.0);
        assert_eq!(summary.max_drawdown, 0.0);
        assert_eq!(summary.total_return_pct, 0.0);
    }

    #[test]
    fn single_win() {
        let summary = analyse(&[buy(100.0), sell(120.0, 20.0)], 100_000.0, 100_020.0, 50);
        assert_eq!(summary.closed_trades, 1);
        assert_eq!(summary.winning_trades, 1);
        assert_eq!(summary.win_rate_pct, 100.0);
        assert_eq!(summary.gross_profit, 20.0);
        assert_eq!(summary.max_drawdown, 0.0);
    }

    #[test]
    fn single_loss() {
        let summary = analyse(&[buy(100.0), sell(80.0, -20.0)], 100_000.0, 99_980.0, 50);
        assert_eq!(summary.losing_trades, 1);
        assert_eq!(summary.win_rate_pct, 0.0);
        assert_eq!(summary.gross_loss, -20.0);
        assert_eq!(summary.largest_loss, -20.0);
        assert_eq!(summary.max_drawdown, 20.0);
    }

    #[test]
    fn drawdown_win_then_loss() {
        let trades = vec![
            buy(100.0),
            sell(200.0, 100.0),
            buy(200.0),
            sell(140.0, -60.0),
        ];
        let summary = analyse(&trades, 100_000.0, 100_040.0, 100);
        assert_eq!(summary.max_drawdown, 60.0);
        assert_eq!(summary.total_realized_pnl, 40.0);
    }

    #[test]
    fn profit_factor() {
        let trades = vec![
            buy(100.0),
            sell(150.0, 50.0),
            buy(100.0),
            sell(130.0, 30.0),
            buy(100.0),
            sell(80.0, -20.0),
            buy(100.0),
            sell(90.0, -10.0),
        ];
        let summary = analyse(&trades, 100_000.0, 100_050.0, 200);
        assert!((summary.profit_factor - 80.0 / 30.0).abs() < 0.001);
    }

    #[test]
    fn consecutive_streaks() {
        let pnls = [10.0, 10.0, 10.0, -5.0, -5.0, 10.0, -5.0, 10.0, 10.0];
        let mut trades = Vec::new();
        for &pnl in &pnls {
            trades.push(buy(100.0));
            trades.push(sell(if pnl > 0.0 { 110.0 } else { 95.0 }, pnl));
        }
        let summary = analyse(&trades, 100_000.0, 100_045.0, 400);
        assert_eq!(summary.max_consecutive_wins, 3);
        assert_eq!(summary.max_consecutive_losses, 2);
    }

    #[test]
    fn breakeven_included_in_closed_not_win_or_loss() {
        let trades = vec![
            buy(100.0),
            sell(110.0, 10.0),
            buy(100.0),
            sell(100.0, 0.0),
            buy(100.0),
            sell(90.0, -10.0),
        ];
        let summary = analyse(&trades, 100_000.0, 100_000.0, 150);
        assert_eq!(summary.closed_trades, 3);
        assert_eq!(summary.winning_trades, 1);
        assert_eq!(summary.breakeven_trades, 1);
        assert_eq!(summary.losing_trades, 1);
        assert!((summary.win_rate_pct - 33.333).abs() < 0.01);
    }

    #[test]
    fn only_buys_no_closed() {
        let summary = analyse(&[buy(100.0), buy(200.0)], 100_000.0, 70_000.0, 50);
        assert_eq!(summary.buy_count, 2);
        assert_eq!(summary.closed_trades, 0);
        assert_eq!(summary.max_drawdown, 0.0);
        assert_eq!(summary.candles_per_trade, 0.0);
    }

    #[test]
    fn total_return_pct() {
        let summary = analyse(&[], 100_000.0, 110_000.0, 0);
        assert!((summary.total_return_pct - 10.0).abs() < 0.001);
    }

    #[test]
    fn candles_per_trade() {
        let trades = (0..4)
            .flat_map(|_| vec![buy(100.0), sell(110.0, 10.0)])
            .collect::<Vec<_>>();
        let summary = analyse(&trades, 100_000.0, 100_040.0, 100);
        assert_eq!(summary.candles_per_trade, 25.0);
    }
}
