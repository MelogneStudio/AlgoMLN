use crate::commands::strategy::run_backtest_internal;
use crate::models::Candle;
use crate::strategy::dsl::{Lexer, Parser, StrategyNode};

fn candle(close: f64) -> Candle {
    Candle {
        timestamp: close as i64,
        open: close,
        high: close,
        low: close,
        close,
        volume: 1000.0,
    }
}

fn parse_strategy(source: &str) -> StrategyNode {
    let tokens = Lexer::tokenize(source).unwrap();
    Parser::new(tokens).parse().unwrap()
}

#[tokio::test]
async fn backtest_known_output() {
    let strategy = parse_strategy("WHEN close > 105\nBUY 1");
    let candles: Vec<Candle> = (100..=108).map(|close| candle(close as f64)).collect();

    let result = run_backtest_internal(strategy, "TEST".to_string(), candles, 100_000.0)
        .await
        .unwrap();

    assert_eq!(result.trade_history.len(), 1);
    assert_eq!(result.total_candles_processed, 9);
}

#[tokio::test]
async fn backtest_is_deterministic() {
    let source = "WHEN close > 105\nBUY 1\n\nWHEN close < 102\nSELL ALL";
    let candles: Vec<Candle> = (98..=110).map(|close| candle(close as f64)).collect();
    let strategy1 = parse_strategy(source);
    let strategy2 = parse_strategy(source);

    let result1 = run_backtest_internal(strategy1, "TEST".to_string(), candles.clone(), 100_000.0)
        .await
        .unwrap();
    let result2 = run_backtest_internal(strategy2, "TEST".to_string(), candles, 100_000.0)
        .await
        .unwrap();

    assert_eq!(result1.trade_history.len(), result2.trade_history.len());
    assert_eq!(result1.total_realized_pnl, result2.total_realized_pnl);
    assert_eq!(result1.final_cash, result2.final_cash);

    for (trade1, trade2) in result1
        .trade_history
        .iter()
        .zip(result2.trade_history.iter())
    {
        assert_eq!(trade1.price, trade2.price);
        assert_eq!(trade1.quantity, trade2.quantity);
        assert_eq!(trade1.side, trade2.side);
    }
}

#[tokio::test]
async fn backtest_buy_sell_pnl_is_correct() {
    let source = "WHEN close > 105\nBUY 1\n\nWHEN close < 95\nSELL ALL";
    let strategy = parse_strategy(source);
    let candles = vec![candle(100.0), candle(110.0), candle(90.0)];

    let result = run_backtest_internal(strategy, "TEST".to_string(), candles, 100_000.0)
        .await
        .unwrap();

    assert_eq!(result.trade_history.len(), 2);
    assert_eq!(result.total_realized_pnl, -20.0);
}
