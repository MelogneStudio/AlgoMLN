use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::broker::{BrokerClient, Timeframe};
use crate::plugin::types::{PluginError, PluginResult, SubscriptionHandle};

use super::{Candle, MarketDataApi, MarketDataEvent, MarketEventKind};

pub struct BrokerMarketDataApi {
    pub broker: Arc<dyn BrokerClient>,
    subscriptions: Arc<Mutex<HashMap<SubscriptionHandle, tokio::task::AbortHandle>>>,
}

impl BrokerMarketDataApi {
    pub fn new(broker: Arc<dyn BrokerClient>) -> Self {
        Self {
            broker,
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[async_trait::async_trait]
impl MarketDataApi for BrokerMarketDataApi {
    fn subscribe_ticks(
        &self,
        symbol: &str,
        callback: Arc<dyn Fn(MarketDataEvent) + Send + Sync>,
    ) -> PluginResult<SubscriptionHandle> {
        let handle = SubscriptionHandle(uuid::Uuid::new_v4());
        let broker = self.broker.clone();
        let sym = symbol.to_string();

        let abort = tokio::spawn(async move {
            loop {
                let timeframe = Timeframe::M1;
                let now = chrono::Utc::now().timestamp();
                let from = now - 60;
                let to = now;
                let sym_for_tick = sym.clone();
                let cb = callback.clone();
                let b = broker.clone();
                // Build a tick event and invoke the callback.
                let event = MarketDataEvent {
                    symbol: sym_for_tick.clone(),
                    kind: MarketEventKind::Tick,
                };
                cb(event);
                // Also push a Candle event for the most recent candle if available.
                let candles = b.get_ohlcv(&sym_for_tick, timeframe, from, to).await;
                if let Ok(candles) = candles {
                    if let Some(last) = candles.last() {
                        let evt = MarketDataEvent {
                            symbol: sym_for_tick,
                            kind: MarketEventKind::Candle,
                        };
                        // Note: the trait event does not carry the candle body — the spec keeps
                        // it as a tagged union over MarketEventKind. The latest candle can
                        // still be retrieved via the latest_candle accessor.
                        let _ = last;
                        cb(evt);
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        })
        .abort_handle();

        // SAFETY: requires a multi-thread Tokio runtime to be current on this thread.
        let map_for_insert = self.subscriptions.clone();
        let h = handle;
        let a = abort;
        let inserted = tokio::runtime::Handle::current().block_on(async move {
            let mut guard = map_for_insert.lock().await;
            guard.insert(h, a).is_some()
        });
        let _ = inserted;

        Ok(handle)
    }

    fn unsubscribe_ticks(&self, handle: SubscriptionHandle) -> PluginResult<()> {
        // SAFETY: requires a multi-thread Tokio runtime to be current on this thread.
        let map = self.subscriptions.clone();
        let removed = tokio::runtime::Handle::current().block_on(async move {
            let mut guard = map.lock().await;
            guard.remove(&handle)
        });
        match removed {
            Some(abort) => {
                abort.abort();
                Ok(())
            }
            None => Err(PluginError::NotFound("subscription not found".into())),
        }
    }

    fn latest_candle(&self, symbol: &str) -> PluginResult<Candle> {
        let broker = self.broker.clone();
        let sym = symbol.to_string();
        // SAFETY: requires a multi-thread Tokio runtime to be current on this thread.
        let res: anyhow::Result<Vec<crate::models::Candle>> = tokio::runtime::Handle::current()
            .block_on(async move {
                let timeframe = Timeframe::M1;
                let now = chrono::Utc::now().timestamp();
                broker.get_ohlcv(&sym, timeframe, now - 60, now).await
            });
        let candles = res.map_err(|e| PluginError::ApiError(e.to_string()))?;
        let last = candles
            .last()
            .ok_or_else(|| PluginError::ApiError("no candles returned".into()))?;
        Ok(Candle {
            open: last.open,
            high: last.high,
            low: last.low,
            close: last.close,
            volume: last.volume,
            timestamp_ms: last.timestamp,
        })
    }
}
