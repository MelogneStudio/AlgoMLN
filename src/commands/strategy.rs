use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use serde::Serialize;

use crate::broker::Timeframe;
use crate::commands::data::DataState;
use crate::data::load_nifty_candles;
use crate::models::Candle;
use crate::strategy::analytics::{BacktestAnalyser, BacktestSummary};
use crate::strategy::dsl::{AstValidator, Lexer, Parser, StrategyNode};
use crate::strategy::execution::{PaperBroker, PaperBrokerState, PaperTrade};
use crate::strategy::logging::LogEntry;
use crate::strategy::runtime::engine::StrategyEngineProfile;
use crate::strategy::runtime::indicator_provider::IndicatorProviderProfile;
use crate::strategy::runtime::{StrategyEngine, StrategyInstance, StrategyStatus};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestResult {
    pub total_candles_processed: usize,
    pub final_cash: f64,
    pub initial_cash: f64,
    pub total_realized_pnl: f64,
    pub trade_history: Vec<PaperTrade>,
    pub broker_state: PaperBrokerState,
    pub logs: Vec<LogEntry>,
    pub summary: BacktestSummary,
    pub profile: BacktestProfile,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestProfile {
    pub total_runtime_ms: u128,
    pub parser_validator_ms: u128,
    pub engine: EngineProfileReport,
    pub indicators: IndicatorProfileReport,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineProfileReport {
    pub on_candle_calls: u64,
    pub on_candle_time_ms: u128,
    pub broker_execute_calls: u64,
    pub broker_execute_time_ms: u128,
    pub broker_get_positions_calls: u64,
    pub broker_get_positions_time_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndicatorProfileReport {
    pub get_calls: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub get_time_ms: u128,
}

pub async fn run_backtest_internal(
    node: StrategyNode,
    symbol: String,
    candles: Vec<Candle>,
    initial_cash: f64,
) -> Result<BacktestResult, String> {
    let total_started = Instant::now();
    let validation_started = Instant::now();
    let validation_errors = AstValidator::validate(&node);
    if !validation_errors.is_empty() {
        return Err(format!("strategy validation failed: {validation_errors:?}"));
    }
    let parser_validator_time = validation_started.elapsed();

    let broker = Arc::new(PaperBroker::new(symbol.clone(), initial_cash));
    let instance = StrategyInstance {
        id: "backtest-strategy".to_string(),
        strategy: Arc::new(node),
        symbol: symbol.clone(),
        timeframe: Timeframe::M5,
        status: StrategyStatus::Running,
        execution_target: broker.clone(),
    };
    let mut engine = StrategyEngine::new(instance);
    // EventBus is not wired during backtests — plugins observe live/paper sessions only.
    // engine.event_bus remains None.
    let mut logs = Vec::new();

    for index in 1..=candles.len() {
        if index % 10_000 == 0 || index == candles.len() {
            eprintln!(
                "Processed {} / {} candles | Elapsed: {:.2}s",
                index,
                candles.len(),
                total_started.elapsed().as_secs_f64()
            );
        }
        broker.update_unrealized(&symbol, candles[index - 1].close);
        logs.extend(engine.on_candle(&candles[..index]).await);
    }

    let engine_profile = engine.profile();
    let indicator_profile = engine.indicator_profile();
    let broker_state = broker.get_state();
    let summary = BacktestAnalyser::analyse(
        &broker_state.trade_history,
        initial_cash,
        broker_state.cash,
        candles.len(),
        &logs,
    );
    Ok(BacktestResult {
        total_candles_processed: candles.len(),
        final_cash: broker_state.cash,
        initial_cash,
        total_realized_pnl: broker_state.total_realized_pnl,
        trade_history: broker_state.trade_history.clone(),
        broker_state,
        logs,
        summary,
        profile: BacktestProfile {
            total_runtime_ms: total_started.elapsed().as_millis(),
            parser_validator_ms: parser_validator_time.as_millis(),
            engine: engine_profile_report(engine_profile),
            indicators: indicator_profile_report(indicator_profile),
        },
    })
}

fn engine_profile_report(profile: StrategyEngineProfile) -> EngineProfileReport {
    EngineProfileReport {
        on_candle_calls: profile.on_candle_calls,
        on_candle_time_ms: profile.on_candle_time.as_millis(),
        broker_execute_calls: profile.broker_execute_calls,
        broker_execute_time_ms: profile.broker_execute_time.as_millis(),
        broker_get_positions_calls: profile.broker_get_positions_calls,
        broker_get_positions_time_ms: profile.broker_get_positions_time.as_millis(),
    }
}

fn indicator_profile_report(profile: IndicatorProviderProfile) -> IndicatorProfileReport {
    IndicatorProfileReport {
        get_calls: profile.get_calls,
        cache_hits: profile.cache_hits,
        cache_misses: profile.cache_misses,
        get_time_ms: profile.get_time.as_millis(),
    }
}

// =============================================================================
// Tauri-facing wire types and DSL orchestrator
// =============================================================================

/// The shape the UI's `BacktestResult` interface expects. The internal
/// `BacktestResult` carries profile/log fields the UI doesn't render; this
/// truncated form keeps the IPC payload small and the contract explicit.
///
/// See `src/types/backtest.ts` for the matching TypeScript interface.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BacktestResultWire {
    pub trade_history: Vec<PaperTradeWire>,
    pub final_cash: f64,
    pub initial_cash: f64,
    pub total_realized_pnl: f64,
    pub total_candles_processed: usize,
    pub summary: BacktestSummary,
}

/// Wire-format `PaperTrade` with `timestamp` rendered as a millisecond string
/// to match the TS type. The internal `PaperTrade.timestamp: i64` is left
/// unchanged for analytics/calculations.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperTradeWire {
    pub id: String,
    pub timestamp: String,
    pub symbol: String,
    pub side: crate::models::OrderSide,
    pub quantity: usize,
    pub price: f64,
    pub pnl: Option<f64>,
}

impl From<&PaperTrade> for PaperTradeWire {
    fn from(trade: &PaperTrade) -> Self {
        Self {
            id: trade.id.clone(),
            timestamp: trade.timestamp.to_string(),
            symbol: trade.symbol.clone(),
            side: trade.side,
            quantity: trade.quantity,
            price: trade.price,
            pnl: trade.pnl,
        }
    }
}

/// Lexes, parses, and validates a `.algomln` source string. Returns a list of
/// human-readable error messages; an empty list means the strategy is valid.
///
/// The UI's `validateDsl` returns `string[]`, so we collapse lex/parse/validate
/// errors into one array. Lex/parse errors are formatted as
/// `"line {line} col {col}: {message}"`.
pub fn validate_dsl(dsl_source: &str) -> Vec<String> {
    let mut errors = Vec::new();

    let tokens = match Lexer::tokenize(dsl_source) {
        Ok(tokens) => tokens,
        Err(error) => {
            errors.push(format!(
                "line {} col {}: {}",
                error.line, error.col, error.message
            ));
            return errors;
        }
    };

    let node = match Parser::new(tokens).parse() {
        Ok(node) => node,
        Err(error) => {
            errors.push(format!(
                "line {} col {}: {}",
                error.line, error.col, error.message
            ));
            return errors;
        }
    };

    for error in AstValidator::validate(&node) {
        errors.push(error.message);
    }
    errors
}

/// Orchestrates a full backtest from raw DSL text: lex/parse/validate, fetch
/// candles (Dhan first, fallback CSV), run the engine, map to wire format.
///
/// `symbol` follows Dhan's `security_id|exchange_segment|instrument` form when
/// a `|` is present; otherwise the broker's default parsing applies. The
/// fallback CSV is always the bundled NIFTY 1-min sample regardless of symbol.
pub async fn run_backtest_dsl(
    dsl_source: &str,
    symbol: &str,
    initial_cash: f64,
    data: &DataState,
) -> Result<BacktestResultWire, String> {
    let tokens = Lexer::tokenize(dsl_source).map_err(|error| {
        format!(
            "line {} col {}: {}",
            error.line, error.col, error.message
        )
    })?;
    let node = Parser::new(tokens).parse().map_err(|error| {
        format!(
            "line {} col {}: {}",
            error.line, error.col, error.message
        )
    })?;

    let validation_errors = AstValidator::validate(&node);
    if !validation_errors.is_empty() {
        let messages = validation_errors
            .iter()
            .map(|error| error.message.clone())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!("strategy validation failed: {messages}"));
    }

    let (candles, used_fallback) = fetch_candles(symbol, data).await?;
    if used_fallback {
        eprintln!(
            "run_backtest_dsl: using bundled NIFTY sample CSV ({} candles) instead of live data for symbol '{}'",
            candles.len(),
            symbol
        );
    }
    if candles.is_empty() {
        return Err("no candles available — provide a valid symbol or ensure the bundled sample CSV is reachable".to_string());
    }

    let result = run_backtest_internal(node, symbol.to_string(), candles, initial_cash).await?;
    Ok(map_to_wire(&result))
}

async fn fetch_candles(
    symbol: &str,
    data: &DataState,
) -> Result<(Vec<Candle>, bool), String> {
    // Try the live broker with the last 7 days of 1-minute candles. If it
    // returns an error or an empty vec (e.g. invalid security ID, no token,
    // holiday window), fall through to the bundled sample CSV.
    let to = Utc::now().timestamp_millis();
    let from = to - 7 * 24 * 60 * 60 * 1_000;
    let broker_result = data
        .broker
        .get_ohlcv(symbol, Timeframe::M1, from, to)
        .await;

    match broker_result {
        Ok(candles) if !candles.is_empty() => Ok((candles, false)),
        Ok(_) => load_fallback_candles().await.map(|c| (c, true)),
        Err(error) => {
            eprintln!(
                "run_backtest_dsl: live fetch failed ({error}); falling back to bundled CSV"
            );
            load_fallback_candles().await.map(|c| (c, true))
        }
    }
}

async fn load_fallback_candles() -> Result<Vec<Candle>, String> {
    // The Tauri binary's CWD is the project root in dev; try both relative
    // shapes that show up across `cargo run`, `npm run tauri dev`, and the
    // packaged binary.
    for candidate in [
        Path::new("sample-data/nifty_1min.csv"),
        Path::new("../sample-data/nifty_1min.csv"),
    ] {
        if candidate.exists() {
            return load_nifty_candles(candidate);
        }
    }
    Err("could not locate sample-data/nifty_1min.csv (tried ./ and ../)".to_string())
}

fn map_to_wire(result: &BacktestResult) -> BacktestResultWire {
    BacktestResultWire {
        trade_history: result.trade_history.iter().map(PaperTradeWire::from).collect(),
        final_cash: result.final_cash,
        initial_cash: result.initial_cash,
        total_realized_pnl: result.total_realized_pnl,
        total_candles_processed: result.total_candles_processed,
        summary: result.summary.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_dsl_returns_empty_for_valid_strategy() {
        let source = "WHEN rsi(14) < 30\nBUY 1\n\nWHEN rsi(14) > 70\nSELL ALL";
        assert!(validate_dsl(source).is_empty());
    }

    #[test]
    fn validate_dsl_catches_lex_error() {
        let source = "WHEN rsi(14) <";
        let errors = validate_dsl(source);
        assert!(!errors.is_empty(), "expected at least one error");
        assert!(
            errors[0].contains("line") && errors[0].contains("col"),
            "expected line/col prefix, got {:?}",
            errors[0]
        );
    }

    #[test]
    fn validate_dsl_catches_validation_error() {
        // Zero quantity is rejected by the validator.
        let source = "WHEN rsi(14) < 30\nBUY 0";
        let errors = validate_dsl(source);
        assert!(errors.iter().any(|m| m.contains("quantity")));
    }

    #[test]
    fn wire_paper_trade_converts_timestamp_to_string() {
        let trade = PaperTrade {
            id: "t1".to_string(),
            timestamp: 1_700_000_000_000,
            symbol: "NIFTY".to_string(),
            side: crate::models::OrderSide::Buy,
            quantity: 10,
            price: 100.0,
            rule_id: "rule_0".to_string(),
            pnl: None,
        };
        let wire = PaperTradeWire::from(&trade);
        assert_eq!(wire.timestamp, "1700000000000");
        assert_eq!(wire.side, crate::models::OrderSide::Buy);
    }
}
