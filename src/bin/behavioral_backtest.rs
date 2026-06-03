use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::{Duration, Instant};

use algomln::commands::strategy::{run_backtest_internal, BacktestResult};
use algomln::models::{Candle, OrderSide};
use algomln::strategy::dsl::{AstValidator, Lexer, Parser, StrategyNode};
use anyhow::{Context, Result};
use chrono::NaiveDateTime;

const INITIAL_CASH: f64 = 10_000_000.0;

struct RunArgs {
    script_path: String,
    data_path: String,
    candle_limit: Option<usize>,
    initial_cash: f64,
    symbol: String,
}

fn main() {
    if let Err(error) = real_main() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<(), String> {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() >= 2 && matches!(args[1].as_str(), "--help" | "-h") {
        print_help();
        return Ok(());
    }

    if args.len() >= 2 && args[1] == "profile" {
        let strategy_name = args.get(2).map(String::as_str).unwrap_or("rsi");
        let limit = args
            .get(3)
            .map(|value| {
                value
                    .parse::<usize>()
                    .map_err(|error| format!("Error: invalid candles value '{value}': {error}"))
            })
            .transpose()?;
        return block_on(run_profile(strategy_name, limit))
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string());
    }

    if args.len() >= 2 && args[1] == "run" {
        return parse_run_args(&args[2..]).and_then(run_script);
    }

    let tiny_strategy = std::fs::read_to_string("sample-data/tiny_strategy.algomln")
        .context("read tiny strategy")
        .map_err(|error| error.to_string())?;
    let tiny_candles =
        load_tiny_candles("sample-data/tiny_candles.csv").map_err(|error| error.to_string())?;

    block_on(run_named(
        "tiny-close-gt-105",
        &tiny_strategy,
        tiny_candles.clone(),
    ))
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    block_on(run_named(
        "idiot-close-gt-0",
        "WHEN close > 0\nBUY 1",
        tiny_candles.clone(),
    ))
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    block_on(run_named(
        "reset-close-gt-105",
        "WHEN close > 105\nBUY 1",
        vec![
            candle(1, 100.0),
            candle(2, 106.0),
            candle(3, 100.0),
            candle(4, 106.0),
        ],
    ))
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;

    Ok(())
}

fn block_on<F>(future: F) -> Result<F::Output>
where
    F: std::future::Future,
{
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("create tokio runtime")
        .map(|runtime| runtime.block_on(future))
}

fn parse_run_args(args: &[String]) -> Result<RunArgs, String> {
    let script_path = args
        .first()
        .cloned()
        .ok_or_else(|| "Error: missing .algomln file for run subcommand".to_string())?;
    let mut data_path = None;
    let mut candle_limit = None;
    let mut initial_cash = 100_000.0;
    let mut symbol = None;
    let mut index = 1;

    while index < args.len() {
        match args[index].as_str() {
            "--data" => {
                index += 1;
                data_path = Some(
                    args.get(index)
                        .cloned()
                        .ok_or_else(|| "Error: --data requires a path".to_string())?,
                );
            }
            "--candles" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "Error: --candles requires a value".to_string())?;
                candle_limit = Some(value.parse::<usize>().map_err(|error| {
                    format!("Error: invalid --candles value '{value}': {error}")
                })?);
            }
            "--cash" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "Error: --cash requires a value".to_string())?;
                initial_cash = value
                    .parse::<f64>()
                    .map_err(|error| format!("Error: invalid --cash value '{value}': {error}"))?;
            }
            "--symbol" => {
                index += 1;
                symbol = Some(
                    args.get(index)
                        .cloned()
                        .ok_or_else(|| "Error: --symbol requires a value".to_string())?,
                );
            }
            other => return Err(format!("Error: unknown run option: {other}")),
        }
        index += 1;
    }

    let data_path =
        data_path.ok_or_else(|| "Error: --data is required for run subcommand".to_string())?;
    let symbol = symbol.unwrap_or_else(|| {
        Path::new(&data_path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("UNKNOWN")
            .to_string()
    });

    Ok(RunArgs {
        script_path,
        data_path,
        candle_limit,
        initial_cash,
        symbol,
    })
}

fn run_script(args: RunArgs) -> Result<(), String> {
    let script_path = Path::new(&args.script_path);
    let extension = script_path
        .extension()
        .and_then(|extension| extension.to_str());
    if extension != Some("algomln") {
        let got = extension
            .map(|extension| format!(".{extension}"))
            .unwrap_or_else(|| "<none>".to_string());
        return Err(format!("Error: expected .algomln file, got {got}"));
    }

    let source = std::fs::read_to_string(&args.script_path)
        .map_err(|_| format!("Error: file not found: {}", args.script_path))?;
    let strategy_name = script_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("strategy")
        .to_string();
    let tokens = Lexer::tokenize(&source).map_err(|error| {
        format!(
            "Error: parse failed at line {} col {}: {}",
            error.line, error.col, error.message
        )
    })?;
    let mut strategy = Parser::new(tokens).parse().map_err(|error| {
        format!(
            "Error: parse failed at line {} col {}: {}",
            error.line, error.col, error.message
        )
    })?;
    strategy.name = strategy_name.clone();

    let validation_errors = AstValidator::validate(&strategy);
    if !validation_errors.is_empty() {
        let mut message = String::from("Error: validation failed:");
        for error in validation_errors {
            let rule_id = if error.rule_id.is_empty() {
                "strategy"
            } else {
                &error.rule_id
            };
            message.push_str(&format!("\n  {rule_id}: {}", error.message));
        }
        return Err(message);
    }

    let mut candles = load_nifty_candles(&args.data_path).map_err(|error| {
        format!(
            "Error: failed to load candles from {}: {error}",
            args.data_path
        )
    })?;
    if let Some(limit) = args.candle_limit {
        candles.truncate(limit);
    }

    println!("strategy: {strategy_name}");
    println!("rules: {}", strategy.rules.len());
    println!("candles: {}", candles.len());
    println!("cash: {:.2}", args.initial_cash);
    println!("symbol: {}", args.symbol);

    let started = Instant::now();
    let result = block_on(run_backtest_internal(
        strategy,
        args.symbol,
        candles,
        args.initial_cash,
    ))
    .map_err(|error| format!("Error: {error}"))?
    .map_err(|error| format!("Error: {error}"))?;

    print_run_summary(&strategy_name, &result, started.elapsed());
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
        result.profile.engine.broker_execute_time_ms, result.profile.engine.broker_execute_calls
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

fn print_run_summary(name: &str, result: &BacktestResult, runtime: Duration) {
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

    for trade in &result.trade_history {
        let side = match trade.side {
            OrderSide::Buy => "BUY",
            OrderSide::Sell => "SELL",
        };
        println!(
            "trade {} {} qty={} price={:.2}",
            trade.id, side, trade.quantity, trade.price
        );
    }

    if result.trade_history.is_empty() {
        println!("no trades executed");
    }
}

fn print_help() {
    println!(
        r#"behavioral_backtest - AlgoMLN strategy runner

USAGE:
  behavioral_backtest                                      run built-in test suite
  behavioral_backtest profile <name> [candles]            run named profile (rsi, ema)
  behavioral_backtest run <file.algomln> --data <csv>     run your own strategy

OPTIONS for run:
  --data <path>      path to OHLCV CSV file (required)
  --candles <n>      limit to first N candles
  --cash <amount>    starting cash (default: 100000)
  --symbol <name>    symbol name for display

EXAMPLES:
  behavioral_backtest run strategies/ema_cross.algomln --data sample-data/nifty_1min.csv
  behavioral_backtest run my_strat.algomln --data sample-data/nifty_1min.csv --candles 50000 --cash 500000
  behavioral_backtest profile rsi 10000"#
    );
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
