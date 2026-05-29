use std::sync::Arc;

use tokio::sync::Mutex;

use crate::{
    broker::{dhan::DhanClient, BrokerClient, Timeframe},
    feed::FeedManager,
    models::{Candle, Quote},
};

pub struct DataState {
    pub broker: Arc<dyn BrokerClient>,
    pub feed: Arc<Mutex<FeedManager>>,
}

impl DataState {
    pub fn dhan_from_env() -> anyhow::Result<Self> {
        Ok(Self {
            broker: Arc::new(DhanClient::from_env()?),
            feed: Arc::new(Mutex::new(FeedManager::new())),
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
