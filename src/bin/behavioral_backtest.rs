use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::time::{Duration, Instant};

use algomln::broker::dhan::{DhanAuth, DhanClient};
use algomln::broker::{BrokerClient, Timeframe};
use algomln::commands::strategy::{run_backtest_internal, BacktestResult};
use algomln::data::load_nifty_candles;
use algomln::models::{Candle, OrderSide};
use algomln::strategy::dsl::{AstValidator, Lexer, Parser, StrategyNode};
use algomln::strategy::logging::LogEntryKind;
use anyhow::{Context, Result};
use chrono::{NaiveDate, Utc};

const INITIAL_CASH: f64 = 10_000_000.0;
const DEFAULT_EXCHANGE_SEGMENT: &str = "NSE_EQ";
const DEFAULT_INSTRUMENT: &str = "EQUITY";
const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

struct RunArgs {
    script_path: String,
    data_path: String,
    candle_limit: Option<usize>,
    initial_cash: f64,
    symbol: String,
}

struct BacktestArgs {
    script_path: String,
    security_id: String,
    exchange_segment: String,
    instrument: String,
    timeframe: Timeframe,
    from: i64,
    to: i64,
    initial_cash: f64,
}

fn main() {
    dotenvy::dotenv().ok();
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

    if args.len() >= 2 && args[1] == "backtest" {
        let args = parse_backtest_args(&args[2..])?;
        return block_on(run_backtest_from_dhan(args)).map_err(|error| error.to_string())?;
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

fn parse_backtest_args(args: &[String]) -> Result<BacktestArgs, String> {
    let script_path = args
        .first()
        .cloned()
        .ok_or_else(|| "Error: missing .algomln file for backtest subcommand".to_string())?;
    let mut security_id = None;
    let mut symbol = None;
    let mut exchange_segment = DEFAULT_EXCHANGE_SEGMENT.to_string();
    let mut instrument = DEFAULT_INSTRUMENT.to_string();
    let mut timeframe = Timeframe::M1;
    let now = Utc::now().timestamp_millis();
    let mut from = now - 365 * DAY_MS;
    let mut to = now;
    let mut initial_cash = INITIAL_CASH;
    let mut index = 1;

    while index < args.len() {
        match args[index].as_str() {
            "--security" => {
                index += 1;
                security_id = Some(
                    args.get(index)
                        .cloned()
                        .ok_or_else(|| "Error: --security requires a value".to_string())?,
                );
            }
            "--symbol" => {
                index += 1;
                symbol = Some(
                    args.get(index)
                        .cloned()
                        .ok_or_else(|| "Error: --symbol requires a value".to_string())?,
                );
            }
            "--exchange" => {
                index += 1;
                exchange_segment = args
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "Error: --exchange requires a value".to_string())?;
            }
            "--instrument" => {
                index += 1;
                instrument = args
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "Error: --instrument requires a value".to_string())?;
            }
            "--timeframe" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "Error: --timeframe requires a value".to_string())?;
                timeframe = parse_backtest_timeframe(value)?;
            }
            "--from" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "Error: --from requires a value".to_string())?;
                from = parse_yyyy_mm_dd(value)?;
            }
            "--to" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "Error: --to requires a value".to_string())?;
                to = parse_yyyy_mm_dd(value)?;
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
            other => return Err(format!("Error: unknown argument for backtest: {other}")),
        }
        index += 1;
    }

    if symbol.is_some() && security_id.is_some() {
        return Err("Error: --symbol and --security are mutually exclusive".to_string());
    }

    if let Some(symbol) = symbol {
        let parts = symbol
            .split(['|', ':'])
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        security_id = Some(parts.first().copied().unwrap_or("").to_string());
        if let Some(value) = parts.get(1) {
            exchange_segment = (*value).to_string();
        }
        if let Some(value) = parts.get(2) {
            instrument = (*value).to_string();
        }
    }

    let security_id = security_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Error: --security <id> is required for backtest subcommand".to_string())?;

    Ok(BacktestArgs {
        script_path,
        security_id,
        exchange_segment,
        instrument,
        timeframe,
        from,
        to,
        initial_cash,
    })
}

fn parse_backtest_timeframe(value: &str) -> Result<Timeframe, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1m" => Ok(Timeframe::M1),
        "5m" => Ok(Timeframe::M5),
        "15m" => Ok(Timeframe::M15),
        "30m" => Ok(Timeframe::M30),
        "1h" => Ok(Timeframe::H1),
        "1d" => Ok(Timeframe::D1),
        _ => Err(format!(
            "Error: unknown timeframe '{value}' — expected 1m 5m 15m 30m 1h 1d"
        )),
    }
}

fn parse_yyyy_mm_dd(value: &str) -> Result<i64, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|_| format!("Error: invalid date '{value}' — expected YYYY-MM-DD"))?
        .and_hms_opt(0, 0, 0)
        .ok_or_else(|| format!("Error: invalid date '{value}' — expected YYYY-MM-DD"))
        .map(|date_time| date_time.and_utc().timestamp_millis())
}

fn timeframe_label(timeframe: Timeframe) -> &'static str {
    match timeframe {
        Timeframe::M1 => "1m",
        Timeframe::M5 => "5m",
        Timeframe::M15 => "15m",
        Timeframe::M30 => "30m",
        Timeframe::H1 | Timeframe::M60 => "1h",
        Timeframe::D1 => "1d",
        _ => "unknown",
    }
}

fn date_label(timestamp_ms: i64) -> String {
    chrono::DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .map(|date_time| date_time.date_naive().format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| timestamp_ms.to_string())
}

fn load_strategy_file(script_path: &str) -> Result<(StrategyNode, String), String> {
    let script_path = Path::new(script_path);
    let extension = script_path
        .extension()
        .and_then(|extension| extension.to_str());
    if extension != Some("algomln") {
        let got = extension
            .map(|extension| format!(".{extension}"))
            .unwrap_or_else(|| "<none>".to_string());
        return Err(format!("Error: expected .algomln file, got {got}"));
    }

    let source = std::fs::read_to_string(script_path)
        .map_err(|_| format!("Error: file not found: {}", script_path.display()))?;
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

    Ok((strategy, strategy_name))
}

fn run_script(args: RunArgs) -> Result<(), String> {
    let (strategy, strategy_name) = load_strategy_file(&args.script_path)?;
    let mut candles = load_nifty_candles(Path::new(&args.data_path)).map_err(|error| {
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
    let symbol = args.symbol.clone();
    let result = block_on(run_backtest_internal(
        strategy,
        symbol.clone(),
        candles,
        args.initial_cash,
    ))
    .map_err(|error| format!("Error: {error}"))?
    .map_err(|error| format!("Error: {error}"))?;

    print_run_summary(&strategy_name, &symbol, &result, started.elapsed());
    Ok(())
}

async fn run_backtest_from_dhan(args: BacktestArgs) -> Result<(), String> {
    let (strategy, strategy_name) = load_strategy_file(&args.script_path)?;
    let access_token = std::env::var("DHAN_ACCESS_TOKEN")
        .map_err(|_| "Error: DHAN_ACCESS_TOKEN environment variable not set".to_string())?;
    let dhan_client =
        DhanClient::new(DhanAuth::new(access_token).map_err(|error| format!("Error: {error}"))?);
    let symbol = format!(
        "{}|{}|{}",
        args.security_id, args.exchange_segment, args.instrument
    );

    println!(
        "fetching: {}  {}  {} → {}",
        symbol,
        timeframe_label(args.timeframe),
        date_label(args.from),
        date_label(args.to)
    );
    let fetch_started = Instant::now();
    let candles = dhan_client
        .get_ohlcv(&symbol, args.timeframe, args.from, args.to)
        .await
        .map_err(|error| format!("Error: Dhan fetch failed: {error}"))?;
    let fetch_ms = fetch_started.elapsed().as_millis();
    println!(
        "fetched: {} candles  (fetch_ms={}ms)",
        candles.len(),
        fetch_ms
    );

    if candles.is_empty() {
        return Err("Error: no candles returned — check security ID and date range".to_string());
    }

    println!("strategy: {strategy_name}");
    println!("rules: {}", strategy.rules.len());
    println!("candles: {}", candles.len());
    println!("cash: {:.2}", args.initial_cash);
    println!("symbol: {symbol}");

    let started = Instant::now();
    let result = run_backtest_internal(strategy, symbol.clone(), candles, args.initial_cash)
        .await
        .map_err(|error| format!("Error: {error}"))?;

    print_run_summary(&strategy_name, &symbol, &result, started.elapsed());
    Ok(())
}

async fn run_profile(strategy_name: &str, limit: Option<usize>) -> Result<()> {
    let load_started = Instant::now();
    let mut nifty = load_nifty_candles(Path::new("sample-data/nifty_1min.csv"))
        .map_err(|e| anyhow::anyhow!("load nifty candles: {e}"))?;
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

fn print_run_summary(name: &str, symbol: &str, result: &BacktestResult, _runtime: Duration) {
    let summary = &result.summary;
    let border = "\u{2550}".repeat(48);

    println!("{border}");
    println!(" BACKTEST - {name}");
    println!(
        " Symbol: {}   Candles: {}",
        symbol, summary.total_candles_processed
    );
    println!("{border}");
    println!();

    println!(" CAPITAL");
    println!("   Initial       ₹ {:>15.2}", summary.initial_cash);
    println!("   Final         ₹ {:>15.2}", summary.final_cash);
    println!("   Return          {:>+14.2}%", summary.total_return_pct);
    println!();

    println!(" TRADES");
    println!(
        "   Total         {:>16}   ({} buys, {} sells)",
        summary.total_trades, summary.buy_count, summary.sell_count
    );
    println!("   Closed        {:>16}", summary.closed_trades);
    println!(
        "   Win rate      {:>15.2}%   ({}W / {}L / {}B)",
        summary.win_rate_pct,
        summary.winning_trades,
        summary.losing_trades,
        summary.breakeven_trades
    );

    if summary.closed_trades == 0 {
        println!(" ⚠  No closed trades - strategy never completed a buy->sell cycle.");
    } else {
        println!();
        println!(" PnL");
        println!("   Realized      ₹ {:>+14.2}", summary.total_realized_pnl);
        println!("   Gross profit  ₹ {:>15.2}", summary.gross_profit);
        println!("   Gross loss    ₹ {:>15.2}", summary.gross_loss);
        println!("   Profit factor   {:>14.2}", summary.profit_factor);
        println!("   Avg win       ₹ {:>+14.2}", summary.avg_win);
        println!("   Avg loss      ₹ {:>+14.2}", summary.avg_loss);
        println!("   Largest win   ₹ {:>+14.2}", summary.largest_win);
        println!("   Largest loss  ₹ {:>+14.2}", summary.largest_loss);
        println!("   Expectancy    ₹ {:>+14.2}", summary.expectancy);
        println!();

        println!(" RISK");
        println!(
            "   Max drawdown  ₹ {:>14.2}  ({:.3}%)",
            summary.max_drawdown, summary.max_drawdown_pct
        );
        println!("   Max consec W    {:>14}", summary.max_consecutive_wins);
        println!("   Max consec L    {:>14}", summary.max_consecutive_losses);
    }

    println!();
    println!(" THROUGHPUT");
    println!("   Candles/trade   {:>14.1}", summary.candles_per_trade);
    println!("   Skipped (no pos){:>13}", summary.skipped_no_position);
    println!();
    println!("{border}");

    for entry in &result.logs {
        if let LogEntryKind::OrderFailed { rule_id, error } = &entry.kind {
            println!(
                "order_failed {} timestamp={} error={}",
                rule_id, entry.candle_timestamp, error
            );
        }
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
  behavioral_backtest backtest <file.algomln> --security <id>   fetch from Dhan and backtest

OPTIONS for run:
  --data <path>      path to OHLCV CSV file (required)
  --candles <n>      limit to first N candles
  --cash <amount>    starting cash (default: 100000)
  --symbol <name>    symbol name for display

OPTIONS for backtest:
  --security <id>       Dhan security ID (required, unless --symbol is used)
  --symbol <id|ex|ins>  compact form: security_id|exchange|instrument
  --exchange <seg>      exchange segment (default: NSE_EQ)
  --instrument <type>   instrument type (default: EQUITY)
  --timeframe <tf>      candle timeframe: 1m 5m 15m 30m 1h 1d (default: 1m)
  --from <YYYY-MM-DD>   start date (default: 365 days ago)
  --to <YYYY-MM-DD>     end date (default: today)
  --cash <amount>       starting cash (default: 10000000)

EXAMPLES:
  behavioral_backtest run strategies/ema_cross.algomln --data sample-data/nifty_1min.csv
  behavioral_backtest run my_strat.algomln --data sample-data/nifty_1min.csv --candles 50000 --cash 500000
  behavioral_backtest backtest strategies/rsi_basic.algomln --security 1333 --from 2024-01-01 --to 2024-06-30
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
