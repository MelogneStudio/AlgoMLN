use std::sync::Arc;
use std::time::Instant;

use serde::Serialize;

use crate::broker::Timeframe;
use crate::models::Candle;
use crate::strategy::analytics::{BacktestAnalyser, BacktestSummary};
use crate::strategy::dsl::{AstValidator, StrategyNode};
use crate::strategy::execution::{PaperBroker, PaperBrokerState, PaperTrade};
use crate::strategy::logging::LogEntry;
use crate::strategy::runtime::engine::StrategyEngineProfile;
use crate::strategy::runtime::indicator_provider::IndicatorProviderProfile;
use crate::strategy::runtime::{StrategyEngine, StrategyInstance, StrategyStatus};

#[derive(Debug, Clone, Serialize)]
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
pub struct BacktestProfile {
    pub total_runtime_ms: u128,
    pub parser_validator_ms: u128,
    pub engine: EngineProfileReport,
    pub indicators: IndicatorProfileReport,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineProfileReport {
    pub on_candle_calls: u64,
    pub on_candle_time_ms: u128,
    pub broker_execute_calls: u64,
    pub broker_execute_time_ms: u128,
    pub broker_get_positions_calls: u64,
    pub broker_get_positions_time_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
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
