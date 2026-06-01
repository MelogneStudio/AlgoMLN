use std::sync::Arc;

use serde::Serialize;

use crate::broker::Timeframe;
use crate::models::Candle;
use crate::strategy::dsl::{AstValidator, StrategyNode};
use crate::strategy::execution::{PaperBroker, PaperBrokerState, PaperTrade};
use crate::strategy::logging::LogEntry;
use crate::strategy::runtime::{StrategyEngine, StrategyInstance, StrategyStatus};

#[derive(Debug, Clone, Serialize)]
pub struct BacktestResult {
    pub total_candles_processed: usize,
    pub final_cash: f64,
    pub total_realized_pnl: f64,
    pub trade_history: Vec<PaperTrade>,
    pub broker_state: PaperBrokerState,
    pub logs: Vec<LogEntry>,
}

pub async fn run_backtest_internal(
    node: StrategyNode,
    symbol: String,
    candles: Vec<Candle>,
    initial_cash: f64,
) -> Result<BacktestResult, String> {
    let validation_errors = AstValidator::validate(&node);
    if !validation_errors.is_empty() {
        return Err(format!("strategy validation failed: {validation_errors:?}"));
    }

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
        broker.update_unrealized(&symbol, candles[index - 1].close);
        logs.extend(engine.on_candle(&candles[..index]).await);
    }

    let broker_state = broker.get_state();
    Ok(BacktestResult {
        total_candles_processed: candles.len(),
        final_cash: broker_state.cash,
        total_realized_pnl: broker_state.total_realized_pnl,
        trade_history: broker_state.trade_history.clone(),
        broker_state,
        logs,
    })
}
