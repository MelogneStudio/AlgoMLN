use std::fs::File;
use std::io::{BufRead, BufReader};
use std::time::{Duration, Instant};

use algomln::commands::strategy::{run_backtest_internal, BacktestResult};
use algomln::models::{Candle, OrderSide};
use algomln::strategy::dsl::{Lexer, Parser, StrategyNode};
use anyhow::{Context, Result};
use chrono::NaiveDateTime;

const INITIAL_CASH: f64 = 10_000_000.0;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let tiny_strategy = std::fs::read_to_string("sample-data/tiny_strategy.algomln")
        .context("read tiny strategy")?;
    let tiny_candles = load_tiny_candles("sample-data/tiny_candles.csv")?;

    run_named("tiny-close-gt-105", &tiny_strategy, tiny_candles.clone()).await?;
    run_named("idiot-close-gt-0", "WHEN close > 0\nBUY 1", tiny_candles.clone()).await?;
    run_named(
        "reset-close-gt-105",
        "WHEN close > 105\nBUY 1",
        vec![candle(1, 100.0), candle(2, 106.0), candle(3, 100.0), candle(4, 106.0)],
    )
    .await?;

    let nifty = load_nifty_candles("sample-data/nifty_1min.csv")?;
    println!("loaded_nifty_candles={}", nifty.len());

    let rsi_strategy = "WHEN rsi(14) < 30\nBUY 1\n\nWHEN rsi(14) > 70\nSELL ALL";
    run_named("nifty-rsi-14", rsi_strategy, nifty.clone()).await?;

    let ema_strategy =
        "WHEN cross_above(ema(20), ema(50))\nBUY 1\n\nWHEN cross_below(ema(20), ema(50))\nSELL ALL";
    run_named("nifty-ema-cross", ema_strategy, nifty.clone()).await?;

    let mut deterministic = Vec::new();
    for index in 1..=3 {
        deterministic.push(run_named(
            &format!("determinism-ema-cross-{index}"),
            ema_strategy,
            nifty.clone(),
        )
        .await?);
    }

    let first = &deterministic[0];
    let deterministic_ok = deterministic.iter().all(|result| {
        result.trade_history.len() == first.trade_history.len()
            && result.total_realized_pnl == first.total_realized_pnl
            && result.final_cash == first.final_cash
    });
    println!("determinism_ok={deterministic_ok}");

    Ok(())
}

async fn run_named(name: &str, source: &str, candles: Vec<Candle>) -> Result<BacktestResult> {
    let strategy = parse_strategy(source)?;
    let started = Instant::now();
    let result = run_backtest_internal(strategy, "NIFTY".to_string(), candles, INITIAL_CASH)
        .await
        .with_context(|| format!("run {name}"))?;
    print_summary(name, &result, started.elapsed());
    Ok(result)
}

fn print_summary(name: &str, result: &BacktestResult, runtime: Duration) {
    println!("=== {name} ===");
    println!("candles={}", result.total_candles_processed);
    println!("trades={}", result.trade_history.len());
    println!("final_cash={:.2}", result.final_cash);
    println!("pnl={:.2}", result.total_realized_pnl);
    println!("runtime_ms={}", runtime.as_millis());
    for trade in result.trade_history.iter().take(5) {
        let side = match trade.side {
            OrderSide::Buy => "BUY",
            OrderSide::Sell => "SELL",
        };
        println!(
            "trade {} {} qty={} price={:.2}",
            trade.id, side, trade.quantity, trade.price
        );
    }
    if result.trade_history.len() > 5 {
        println!("... {} more trades", result.trade_history.len() - 5);
    }
}

fn parse_strategy(source: &str) -> Result<StrategyNode> {
    let tokens = Lexer::tokenize(source).map_err(|error| anyhow::anyhow!("{error:?}"))?;
    Parser::new(tokens)
        .parse()
        .map_err(|error| anyhow::anyhow!("{error:?}"))
}

fn load_tiny_candles(path: &str) -> Result<Vec<Candle>> {
    let file = File::open(path).with_context(|| format!("open {path}"))?;
    let mut candles = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if index == 0 || line.trim().is_empty() {
            continue;
        }
        let fields = line.split(',').collect::<Vec<_>>();
        anyhow::ensure!(fields.len() == 6, "bad tiny candle row {}", index + 1);
        candles.push(Candle {
            timestamp: fields[0].parse::<i64>()?,
            open: fields[1].parse::<f64>()?,
            high: fields[2].parse::<f64>()?,
            low: fields[3].parse::<f64>()?,
            close: fields[4].parse::<f64>()?,
            volume: fields[5].parse::<f64>()?,
        });
    }
    Ok(candles)
}

fn load_nifty_candles(path: &str) -> Result<Vec<Candle>> {
    let file = File::open(path).with_context(|| format!("open {path}"))?;
    let mut candles = Vec::new();
    for (index, line) in BufReader::new(file).lines().enumerate() {
        let line = line?;
        if index == 0 || line.trim().is_empty() {
            continue;
        }
        let fields = line.split_whitespace().collect::<Vec<_>>();
        anyhow::ensure!(fields.len() == 6, "bad NIFTY candle row {}", index + 1);
        let datetime = format!("{} {}", fields[0], fields[1]);
        let timestamp = NaiveDateTime::parse_from_str(&datetime, "%Y-%m-%d %H:%M:%S")?
            .and_utc()
            .timestamp_millis();
        candles.push(Candle {
            timestamp,
            open: fields[2].parse::<f64>()?,
            high: fields[3].parse::<f64>()?,
            low: fields[4].parse::<f64>()?,
            close: fields[5].parse::<f64>()?,
            volume: 1_000.0,
        });
    }
    Ok(candles)
}

fn candle(timestamp: i64, close: f64) -> Candle {
    Candle {
        timestamp,
        open: close,
        high: close,
        low: close,
        close,
        volume: 1_000.0,
    }
}
