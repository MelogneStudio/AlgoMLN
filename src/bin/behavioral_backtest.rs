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
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() >= 2 && args[1] == "profile" {
        let strategy_name = args.get(2).map(String::as_str).unwrap_or("rsi");
        let limit = args
            .get(3)
            .map(|value| value.parse::<usize>())
            .transpose()?;
        return run_profile(strategy_name, limit).await;
    }

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

    Ok(())
}

async fn run_profile(strategy_name: &str, limit: Option<usize>) -> Result<()> {
    let load_started = Instant::now();
    let mut nifty = load_nifty_candles("sample-data/nifty_1min.csv")?;
    if let Some(limit) = limit {
        nifty.truncate(limit);
    }
    println!(
        "loaded_nifty_candles={} load_ms={}",
        nifty.len(),
        load_started.elapsed().as_millis()
    );

    let strategy = match strategy_name {
        "rsi" => "WHEN rsi(14) < 30\nBUY 1\n\nWHEN rsi(14) > 70\nSELL ALL",
        "ema" => {
            "WHEN cross_above(ema(20), ema(50))\nBUY 1\n\nWHEN cross_below(ema(20), ema(50))\nSELL ALL"
        }
        other => anyhow::bail!("unknown strategy: {other}"),
    };

    run_named(&format!("nifty-{strategy_name}"), strategy, nifty).await?;
    Ok(())
}

async fn run_named(name: &str, source: &str, candles: Vec<Candle>) -> Result<BacktestResult> {
    let parse_started = Instant::now();
    let strategy = parse_strategy(source)?;
    let parse_time = parse_started.elapsed();
    let started = Instant::now();
    let result = run_backtest_internal(strategy, "NIFTY".to_string(), candles, INITIAL_CASH)
        .await
        .map_err(|error| anyhow::anyhow!("run {name}: {error}"))?;
    print_summary(name, &result, started.elapsed(), parse_time);
    Ok(result)
}

fn print_summary(name: &str, result: &BacktestResult, runtime: Duration, parse_time: Duration) {
    println!("=== {name} ===");
    println!("candles={}", result.total_candles_processed);
    println!("trades={}", result.trade_history.len());
    println!("final_cash={:.2}", result.final_cash);
    println!("pnl={:.2}", result.total_realized_pnl);
    println!("runtime_ms={}", runtime.as_millis());
    println!(
        "candles_per_sec={:.2}",
        result.total_candles_processed as f64 / runtime.as_secs_f64().max(0.001)
    );
    println!("parser_ms={}", parse_time.as_millis());
    println!("validator_ms={}", result.profile.parser_validator_ms);
    println!(
        "engine_on_candle_ms={} calls={}",
        result.profile.engine.on_candle_time_ms, result.profile.engine.on_candle_calls
    );
    println!(
        "indicator_get_ms={} calls={} hits={} misses={}",
        result.profile.indicators.get_time_ms,
        result.profile.indicators.get_calls,
        result.profile.indicators.cache_hits,
        result.profile.indicators.cache_misses
    );
    println!(
        "paper_execute_ms={} calls={}",
        result.profile.engine.broker_execute_time_ms,
        result.profile.engine.broker_execute_calls
    );
    println!(
        "paper_get_positions_ms={} calls={}",
        result.profile.engine.broker_get_positions_time_ms,
        result.profile.engine.broker_get_positions_calls
    );
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
        let fields = parse_market_row(&line)
            .with_context(|| format!("bad NIFTY candle row {}", index + 1))?;
        let timestamp = NaiveDateTime::parse_from_str(fields[0], "%Y-%m-%d %H:%M:%S")?
            .and_utc()
            .timestamp_millis();
        candles.push(Candle {
            timestamp,
            open: fields[1].parse::<f64>()?,
            high: fields[2].parse::<f64>()?,
            low: fields[3].parse::<f64>()?,
            close: fields[4].parse::<f64>()?,
            volume: 1_000.0,
        });
    }
    Ok(candles)
}

fn parse_market_row(line: &str) -> Result<Vec<&str>> {
    let tab_fields = line.split('\t').collect::<Vec<_>>();
    if tab_fields.len() == 5 {
        return Ok(tab_fields);
    }

    let comma_fields = line.split(',').collect::<Vec<_>>();
    if comma_fields.len() == 5 {
        return Ok(comma_fields);
    }

    let whitespace_fields = line.split_whitespace().collect::<Vec<_>>();
    if whitespace_fields.len() == 6 {
        return Ok(vec![
            line.get(0..19).context("missing datetime")?,
            whitespace_fields[2],
            whitespace_fields[3],
            whitespace_fields[4],
            whitespace_fields[5],
        ]);
    }

    anyhow::bail!("expected 5 market columns")
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
