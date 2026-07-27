use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{
    broker::{
        dhan::{DhanAuth, DhanClient},
        symbol_map::SymbolMap,
        BrokerClient, Timeframe,
    },
    feed::FeedManager,
    live::trade_log::TradeLog,
    models::{Candle, Quote},
    strategy::execution::DhanBroker,
};

/// Shared data + execution infrastructure used by every Tauri command.
///
/// `broker` is the trait-object view the rest of the app uses for OHLCV /
/// quote / subscribe calls. `dhan_broker` (Dhan only) is the live execution
/// target plus the periodic realized-loss cache; both wrap the same
/// `Arc<DhanClient>`. The trailing `Option` is for environments where live
/// execution is disabled — currently always `Some` because Phase 7 hard-
/// requires Dhan.
pub struct DataState {
    pub broker: Arc<dyn BrokerClient>,
    pub feed: Arc<Mutex<FeedManager>>,
    pub dhan_broker: Option<Arc<DhanBroker>>,
    pub dhan_client: Option<Arc<DhanClient>>,
}

impl DataState {
    pub fn dhan_from_env() -> anyhow::Result<Self> {
        Self::dhan_from_env_with_symbol_map(
            Arc::new(parking_lot::RwLock::new(SymbolMap::empty())),
            None,
        )
    }

    pub fn dhan_from_env_with_symbol_map(
        symbol_map: Arc<parking_lot::RwLock<SymbolMap>>,
        trade_log: Option<Arc<TradeLog>>,
    ) -> anyhow::Result<Self> {
        let auth = DhanAuth::from_env()?;
        let dhan_client = Arc::new(DhanClient::with_symbol_map(auth, symbol_map));
        // The DhanBroker needs its own realized-loss cache + session context.
        // If the caller hasn't passed a trade log yet, open one against a
        // best-effort default path inside the app data dir — the setup
        // closure normally supplies this, so the `None` branch is only
        // exercised by tests.
        let broker: Arc<dyn BrokerClient> = dhan_client.clone();
        let dhan_broker = trade_log.map(|log| Arc::new(DhanBroker::new(dhan_client.clone(), log)));
        Ok(Self {
            broker,
            feed: Arc::new(Mutex::new(FeedManager::new())),
            dhan_broker,
            dhan_client: Some(dhan_client),
        })
    }
}

pub async fn get_ohlcv(
    state: &DataState,
    symbol: String,
    timeframe: Timeframe,
    from: i64,
    to: i64,
) -> Result<Vec<Candle>, String> {
    state
        .broker
        .get_ohlcv(&symbol, timeframe, from, to)
        .await
        .map_err(|error| error.to_string())
}

pub async fn get_quote(state: &DataState, symbol: String) -> Result<Quote, String> {
    state
        .broker
        .get_quote(&symbol)
        .await
        .map_err(|error| error.to_string())
}

pub async fn subscribe_ticks(state: &DataState, symbols: Vec<String>) -> Result<(), String> {
    state
        .feed
        .lock()
        .await
        .subscribe(symbols)
        .await
        .map_err(|error| error.to_string())
}
