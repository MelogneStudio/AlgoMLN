use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;

use crate::{
    broker::{BrokerClient, Timeframe},
    models::{Candle, Order, OrderResult, Portfolio, Position, Quote},
};

use super::{
    auth::DhanAuth,
    models::{DhanQuoteValue, DhanSymbol, HistoricalRequest, HistoricalResponse, IntradayRequest},
};

const DHAN_BASE_URL: &str = "https://api.dhan.co/v2";
const INTRADAY_CHUNK_MS: i64 = 89 * 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone)]
pub struct DhanConfig {
    pub base_url: String,
    pub default_exchange_segment: String,
    pub default_instrument: String,
}

impl Default for DhanConfig {
    fn default() -> Self {
        Self {
            base_url: DHAN_BASE_URL.to_string(),
            default_exchange_segment: "NSE_EQ".to_string(),
            default_instrument: "EQUITY".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DhanClient {
    http: reqwest::Client,
    auth: DhanAuth,
    config: DhanConfig,
}

impl DhanClient {
    pub fn new(auth: DhanAuth) -> Self {
        Self::with_config(auth, DhanConfig::default())
    }

    pub fn with_config(auth: DhanAuth, config: DhanConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            auth,
            config,
        }
    }

    pub fn from_env() -> Result<Self> {
        Ok(Self::new(DhanAuth::from_env()?))
    }

    fn headers(&self) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();
        headers.insert(
            "access-token",
            HeaderValue::from_str(&self.auth.access_token)
                .context("Dhan access token contains invalid header characters")?,
        );

        Ok(headers)
    }

    fn symbol(&self, symbol: &str) -> DhanSymbol {
        DhanSymbol::parse(
            symbol,
            &self.config.default_exchange_segment,
            &self.config.default_instrument,
        )
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: impl serde::Serialize,
    ) -> Result<T> {
        let response = self
            .http
            .post(format!("{}{}", self.config.base_url, path))
            .headers(self.headers()?)
            .json(&body)
            .send()
            .await
            .with_context(|| format!("Dhan request failed: {path}"))?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("Dhan request {path} failed with {status}: {body}");
        }

        response
            .json::<T>()
            .await
            .with_context(|| format!("Dhan response was not valid JSON: {path}"))
    }

    pub fn dhan_timestamp_to_unix_ms(timestamp: f64) -> Option<i64> {
        if !timestamp.is_finite() {
            return None;
        }

        Some((timestamp as i64) * 1_000)
    }

    pub fn unix_ms_to_dhan_date(timestamp: i64) -> Result<String> {
        let datetime = Utc
            .timestamp_millis_opt(timestamp)
            .single()
            .ok_or_else(|| anyhow!("Invalid Unix millisecond timestamp: {timestamp}"))?;

        Ok(datetime.format("%Y-%m-%d").to_string())
    }

    pub fn unix_ms_to_dhan_datetime(timestamp: i64) -> Result<String> {
        let datetime = Utc
            .timestamp_millis_opt(timestamp)
            .single()
            .ok_or_else(|| anyhow!("Invalid Unix millisecond timestamp: {timestamp}"))?;

        Ok(datetime.format("%Y-%m-%d %H:%M:%S").to_string())
    }

    // Splits a ms timestamp range into 89-day chunks.
    // Returns (chunk_from_ms, chunk_to_ms) pairs.
    fn chunk_date_range(from: i64, to: i64) -> Vec<(i64, i64)> {
        let mut chunks = Vec::new();
        let mut cur = from;
        while cur < to {
            let end = (cur + INTRADAY_CHUNK_MS).min(to);
            chunks.push((cur, end));
            cur = end + 1;
        }
        chunks
    }

    fn candles_from_response(response: HistoricalResponse) -> Vec<Candle> {
        let mut candles = response
            .timestamp
            .into_iter()
            .zip(response.open)
            .zip(response.high)
            .zip(response.low)
            .zip(response.close)
            .zip(response.volume)
            .filter_map(|(((((timestamp, open), high), low), close), volume)| {
                let timestamp = Self::dhan_timestamp_to_unix_ms(timestamp?)?;
                let open = finite(open?)?;
                let high = finite(high?)?;
                let low = finite(low?)?;
                let close = finite(close?)?;
                let volume = finite(volume?)?;

                Some(Candle {
                    timestamp,
                    open,
                    high,
                    low,
                    close,
                    volume,
                })
            })
            .collect::<Vec<_>>();

        candles.sort_by_key(|candle| candle.timestamp);
        candles
    }

    fn quote_from_value(symbol: &str, value: DhanQuoteValue) -> Result<Quote> {
        Ok(Quote {
            symbol: symbol.to_string(),
            ltp: value
                .last_price
                .ok_or_else(|| anyhow!("Quote missing LTP"))?,
            open: value.open.unwrap_or_default(),
            high: value.high.unwrap_or_default(),
            low: value.low.unwrap_or_default(),
            close: value.close.unwrap_or_default(),
            bid: value.bid_price.unwrap_or_default(),
            ask: value.ask_price.unwrap_or_default(),
            volume: value.volume.unwrap_or_default(),
        })
    }

    async fn get_ohlcv_intraday(
        &self,
        symbol: &DhanSymbol,
        timeframe: &Timeframe,
        from: i64,
        to: i64,
    ) -> Result<Vec<Candle>> {
        let interval = timeframe.to_interval_str().to_string();
        let chunks = Self::chunk_date_range(from, to);
        let mut all_candles: Vec<Candle> = Vec::new();

        for (chunk_from, chunk_to) in chunks {
            let body = IntradayRequest {
                security_id: symbol.security_id.clone(),
                exchange_segment: symbol.exchange_segment.clone(),
                instrument: symbol.instrument.clone(),
                interval: interval.clone(),
                oi: false,
                from_date: Self::unix_ms_to_dhan_datetime(chunk_from)?,
                to_date: Self::unix_ms_to_dhan_datetime(chunk_to)?,
            };

            let response = self
                .post::<HistoricalResponse>("/charts/intraday", body)
                .await?;

            all_candles.extend(Self::candles_from_response(response));
        }

        // dedup in case chunk boundaries overlap, then sort
        all_candles.sort_by_key(|c| c.timestamp);
        all_candles.dedup_by_key(|c| c.timestamp);

        Ok(all_candles)
    }
}

#[async_trait]
impl BrokerClient for DhanClient {
    async fn get_ohlcv(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
    ) -> Result<Vec<Candle>> {
        let symbol = self.symbol(symbol);

        if timeframe.is_intraday() {
            return self.get_ohlcv_intraday(&symbol, &timeframe, from, to).await;
        }

        // daily / weekly — existing path
        let body = HistoricalRequest {
            security_id: &symbol.security_id,
            exchange_segment: &symbol.exchange_segment,
            instrument: &symbol.instrument,
            expiry_code: 0,
            from_date: Self::unix_ms_to_dhan_date(from)?,
            to_date: Self::unix_ms_to_dhan_date(to)?,
        };

        let response = self
            .post::<HistoricalResponse>("/charts/historical", body)
            .await?;
        Ok(Self::candles_from_response(response))
    }

    async fn get_quote(&self, symbol: &str) -> Result<Quote> {
        let dhan_symbol = self.symbol(symbol);
        let mut body = serde_json::Map::new();
        body.insert(
            dhan_symbol.exchange_segment.clone(),
            serde_json::json!([dhan_symbol
                .security_id
                .parse::<i64>()
                .context("Dhan security id must be numeric for quotes")?]),
        );

        let response = self
            .post::<Value>("/marketfeed/quote", Value::Object(body))
            .await?;
        let root = response.get("data").unwrap_or(&response);
        let exchange = root
            .get(&dhan_symbol.exchange_segment)
            .ok_or_else(|| anyhow!("Quote response missing exchange segment"))?;
        let numeric_security_id_key = dhan_symbol
            .security_id
            .parse::<i64>()
            .ok()
            .map(|security_id| security_id.to_string());
        let value = exchange
            .get(dhan_symbol.security_id.as_str())
            .or_else(|| exchange.get(numeric_security_id_key.as_deref()?))
            .ok_or_else(|| anyhow!("Quote response missing security id"))?;
        let value = serde_json::from_value::<DhanQuoteValue>(value.clone())?;

        Self::quote_from_value(symbol, value)
    }

    async fn place_order(&self, order: Order) -> Result<OrderResult> {
        let _ = order;
        bail!("Dhan order placement is not implemented yet");
    }

    async fn get_positions(&self) -> Result<Vec<Position>> {
        bail!("Dhan positions are not implemented yet");
    }

    async fn get_portfolio(&self) -> Result<Portfolio> {
        let positions = self.get_positions().await?;
        let total_value = positions
            .iter()
            .map(|position| position.ltp * position.quantity as f64)
            .sum();
        let realized_pnl = positions.iter().map(|position| position.realized_pnl).sum();
        let unrealized_pnl = positions
            .iter()
            .map(|position| position.unrealized_pnl)
            .sum();

        Ok(Portfolio {
            positions,
            total_value,
            realized_pnl,
            unrealized_pnl,
        })
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use std::{env, fs};

    #[test]
    fn converts_dhan_unix_seconds_to_unix_ms() {
        assert_eq!(
            DhanClient::dhan_timestamp_to_unix_ms(1_779_820_200.0),
            Some(1_779_820_200_000)
        );
    }

    #[test]
    fn filters_incomplete_or_nan_candles_and_sorts() {
        let response = HistoricalResponse {
            timestamp: vec![Some(2.0), None, Some(1.0), Some(3.0)],
            open: vec![Some(20.0), Some(1.0), Some(10.0), Some(f64::NAN)],
            high: vec![Some(21.0), Some(1.0), Some(11.0), Some(3.0)],
            low: vec![Some(19.0), Some(1.0), Some(9.0), Some(3.0)],
            close: vec![Some(20.5), Some(1.0), Some(10.5), Some(3.0)],
            volume: vec![Some(200.0), Some(1.0), Some(100.0), Some(3.0)],
        };

        let candles = DhanClient::candles_from_response(response);

        assert_eq!(candles.len(), 2);
        assert_eq!(candles[0].open, 10.0);
        assert_eq!(candles[1].open, 20.0);
    }

    #[test]
    fn converts_unix_ms_to_dhan_date() {
        assert_eq!(
            DhanClient::unix_ms_to_dhan_date(1_704_067_200_000).unwrap(),
            "2024-01-01"
        );
    }

    #[test]
    fn converts_unix_ms_to_dhan_datetime() {
        assert_eq!(
            DhanClient::unix_ms_to_dhan_datetime(1_704_067_200_000).unwrap(),
            "2024-01-01 00:00:00"
        );
    }

    #[test]
    fn chunks_date_range_into_89_day_windows() {
        // 200 days should give 3 chunks: 89 + 89 + 22
        let from = 0i64;
        let to = 200 * 24 * 60 * 60 * 1_000;
        let chunks = DhanClient::chunk_date_range(from, to);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].0, 0);
        assert_eq!(chunks[1].0, INTRADAY_CHUNK_MS + 1);
        assert_eq!(chunks[2].1, to);
    }

    #[test]
    fn single_chunk_when_range_under_89_days() {
        let from = 0i64;
        let to = 10 * 24 * 60 * 60 * 1_000; // 10 days
        let chunks = DhanClient::chunk_date_range(from, to);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], (from, to));
    }

    #[tokio::test]
    #[ignore = "hits the live Dhan API and requires DHAN_ACCESS_TOKEN"]
    async fn live_fetch() {
        load_dotenv_for_test();

        let client = DhanClient::from_env().expect("Set DHAN_ACCESS_TOKEN in .env or the shell");
        let symbol =
            env::var("DHAN_TEST_SYMBOL").unwrap_or_else(|_| "2885|NSE_EQ|EQUITY".to_string());
        let to = Utc::now().timestamp_millis();
        let from = (Utc::now() - Duration::days(30)).timestamp_millis();

        let candles = client
            .get_ohlcv(&symbol, Timeframe::D1, from, to)
            .await
            .expect("live Dhan OHLCV fetch failed");

        println!("Fetched {} candles for {}", candles.len(), symbol);
        if let Some(last) = candles.last() {
            println!("Last candle: {:?}", last);
        }

        assert!(!candles.is_empty(), "Dhan returned no candles");
    }

    fn load_dotenv_for_test() {
        let Ok(contents) = fs::read_to_string(".env") else {
            return;
        };

        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                continue;
            };

            if env::var(key.trim()).is_err() {
                env::set_var(key.trim(), value.trim().trim_matches('"'));
            }
        }
    }
}
