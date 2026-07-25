use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{TimeZone, Utc};
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::Value;
use std::sync::Arc;

use crate::{
    broker::{
        symbol_map::{Segment, SymbolEntry, SymbolMap},
        BrokerClient, Timeframe,
    },
    models::{
        Candle, Order, OrderResult, OrderSide, OrderStatus, OrderType, Portfolio, Position, Quote,
    },
};

use super::{
    auth::DhanAuth,
    models::{
        DhanPosition, DhanQuoteValue, DhanSymbol, HistoricalResponse, IntradayRequest,
        PlaceOrderRequest, PlaceOrderResponse,
    },
};

const DHAN_BASE_URL: &str = "https://api.dhan.co/v2";
const INTRADAY_CHUNK_MS: i64 = 89 * 24 * 60 * 60 * 1_000;
const DHAN_HISTORICAL_EPOCH_OFFSET_SECONDS: i64 = 315_532_800;

/// Errors from the Dhan REST client that callers must handle specifically
/// (rather than as an opaque `anyhow` error). Currently only the
/// order-placement "unknown status" case.
#[derive(Debug, Clone)]
pub enum DhanError {
    /// An order request timed out. The order may or may not have reached the
    /// exchange, so the caller must **not** retry and must **not** assume a
    /// fill — it should tell the user to check their broker app. Carries the
    /// `correlationId` so the user (and support) can locate the order.
    OrderStatusUnknown { correlation_id: String },
}

impl std::fmt::Display for DhanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OrderStatusUnknown { correlation_id } => write!(
                f,
                "order status unknown, check broker app; correlation_id={correlation_id}"
            ),
        }
    }
}

impl std::error::Error for DhanError {}

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
    symbol_map: Arc<parking_lot::RwLock<SymbolMap>>,
}

impl DhanClient {
    pub fn new(auth: DhanAuth) -> Self {
        Self::with_config(auth, DhanConfig::default())
    }

    pub fn with_symbol_map(
        auth: DhanAuth,
        symbol_map: Arc<parking_lot::RwLock<SymbolMap>>,
    ) -> Self {
        Self::with_config_and_symbol_map(auth, DhanConfig::default(), symbol_map)
    }

    pub fn with_config(auth: DhanAuth, config: DhanConfig) -> Self {
        Self::with_config_and_symbol_map(
            auth,
            config,
            Arc::new(parking_lot::RwLock::new(SymbolMap::empty())),
        )
    }

    pub fn with_config_and_symbol_map(
        auth: DhanAuth,
        config: DhanConfig,
        symbol_map: Arc<parking_lot::RwLock<SymbolMap>>,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            auth,
            config,
            symbol_map,
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

    /// Shared POST helper. **Performs exactly one request — no automatic
    /// retry** on timeout, 5xx, or network error. Order placement relies on
    /// this invariant (a retried order can double-fill); see
    /// [`DhanClient::post_order_no_retry`], which additionally distinguishes a
    /// timed-out order as [`DhanError::OrderStatusUnknown`]. Do not add retry
    /// logic here without routing order placement around it.
    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: impl serde::Serialize,
    ) -> Result<T> {
        let request_body =
            serde_json::to_string(&body).unwrap_or_else(|_| "<unserializable>".to_string());
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
            bail!("Dhan request {path} failed with {status}: {body}; request body: {request_body}");
        }

        response
            .json::<T>()
            .await
            .with_context(|| format!("Dhan response was not valid JSON: {path}"))
    }

    /// POST to `/orders` with **no automatic retry**. Order placement is not
    /// idempotent at the network layer: a timed-out request may still have
    /// reached the exchange, so retrying blindly risks a duplicate fill. On a
    /// request timeout this returns [`DhanError::OrderStatusUnknown`] so the
    /// caller can surface "check your broker app" instead of assuming success
    /// or failure. See Phase 7 Fix Pack, Fix 2.
    async fn post_order_no_retry(
        &self,
        body: &PlaceOrderRequest,
        correlation_id: &str,
    ) -> Result<PlaceOrderResponse> {
        let request_body =
            serde_json::to_string(body).unwrap_or_else(|_| "<unserializable>".to_string());
        let response = match self
            .http
            .post(format!("{}/orders", self.config.base_url))
            .headers(self.headers()?)
            .json(body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) if err.is_timeout() => {
                return Err(DhanError::OrderStatusUnknown {
                    correlation_id: correlation_id.to_string(),
                }
                .into());
            }
            Err(err) => {
                return Err(anyhow::Error::new(err).context("Dhan order request failed: /orders"));
            }
        };

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            bail!("Dhan request /orders failed with {status}: {body}; request body: {request_body}");
        }

        response
            .json::<PlaceOrderResponse>()
            .await
            .context("Dhan response was not valid JSON: /orders")
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let response = self
            .http
            .get(format!("{}{}", self.config.base_url, path))
            .headers(self.headers()?)
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

    fn resolve_symbol_entry(&self, symbol: &str) -> Result<SymbolEntry> {
        let symbol_map = self.symbol_map.read();
        symbol_map
            .lookup(symbol)
            .ok_or_else(|| anyhow!("symbol not in map: {symbol}"))
    }

    fn order_status(status: &str) -> OrderStatus {
        match status.trim().to_ascii_uppercase().as_str() {
            "TRANSIT" => OrderStatus::Transit,
            "PENDING" => OrderStatus::Pending,
            "TRADED" | "FILLED" | "COMPLETE" | "EXECUTED" => OrderStatus::Traded,
            "REJECTED" => OrderStatus::Rejected,
            "CANCELLED" | "CANCELED" => OrderStatus::Cancelled,
            "EXPIRED" => OrderStatus::Expired,
            other => OrderStatus::Unknown(other.to_string()),
        }
    }

    pub fn dhan_timestamp_to_unix_ms(timestamp: f64) -> Option<i64> {
        if !timestamp.is_finite() {
            return None;
        }

        Some((timestamp as i64) * 1_000)
    }

    pub fn dhan_historical_timestamp_to_unix_ms(timestamp: f64) -> Option<i64> {
        if !timestamp.is_finite() {
            return None;
        }

        Some(((timestamp as i64) + DHAN_HISTORICAL_EPOCH_OFFSET_SECONDS) * 1_000)
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
        Self::candles_from_response_with_timestamp(response, Self::dhan_timestamp_to_unix_ms)
    }

    fn candles_from_historical_response(response: HistoricalResponse) -> Vec<Candle> {
        Self::candles_from_response_with_timestamp(
            response,
            Self::dhan_historical_timestamp_to_unix_ms,
        )
    }

    fn candles_from_response_with_timestamp(
        response: HistoricalResponse,
        timestamp_to_unix_ms: fn(f64) -> Option<i64>,
    ) -> Vec<Candle> {
        let mut candles = response
            .timestamp
            .into_iter()
            .zip(response.open)
            .zip(response.high)
            .zip(response.low)
            .zip(response.close)
            .zip(response.volume)
            .filter_map(|(((((timestamp, open), high), low), close), volume)| {
                let timestamp = timestamp_to_unix_ms(timestamp?)?;
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
        let body = serde_json::json!({
            "securityId": symbol.security_id,
            "exchangeSegment": symbol.exchange_segment,
            "instrument": symbol.instrument,
            "expiryCode": 0,
            "oi": "false",
            "fromDate": Self::unix_ms_to_dhan_date(from)?,
            "toDate": Self::unix_ms_to_dhan_date(to)?,
        });

        let response = self
            .post::<HistoricalResponse>("/charts/historical", body)
            .await?;
        Ok(Self::candles_from_historical_response(response))
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
        let dhan_client_id = self
            .auth
            .client_id
            .clone()
            .ok_or_else(|| anyhow!("Dhan client id is not configured"))?;
        let entry = self.resolve_symbol_entry(&order.symbol)?;
        // Segment guard (Phase 7 Fix Pack, Fix 3). Defence in depth: even if a
        // pre-flight gate is bypassed, the broker refuses anything that is not
        // NSE equity intraday. Relaxing this is a Phase 8 task.
        if entry.segment != Segment::NseEq {
            bail!(
                "Phase 7 only supports NSE equity intraday trading; symbol {} is {:?}",
                order.symbol,
                entry.segment
            );
        }
        let security_id = entry.security_id.to_string();
        let order_type = match order.order_type {
            OrderType::Market => "MARKET",
            OrderType::Limit => "LIMIT",
            OrderType::StopLoss => "STOP_LOSS",
        };
        let price = match order.order_type {
            OrderType::Limit => order
                .price
                .ok_or_else(|| anyhow!("limit order requires a price"))?,
            _ => order.price.unwrap_or_default(),
        };
        // Idempotency key: if any layer retries a timed-out request, Dhan
        // dedupes on this so the same order is not placed twice.
        let correlation_id = format!("algomln-{}", uuid::Uuid::new_v4());
        let body = PlaceOrderRequest {
            dhan_client_id,
            correlation_id: correlation_id.clone(),
            transaction_type: match order.side {
                OrderSide::Buy => "BUY".to_string(),
                OrderSide::Sell => "SELL".to_string(),
            },
            exchange_segment: entry.segment.as_dhan_string().to_string(),
            product_type: "INTRADAY".to_string(),
            order_type: order_type.to_string(),
            validity: "DAY".to_string(),
            security_id,
            quantity: order.quantity,
            price,
            trigger_price: 0.0,
            disclosed_quantity: 0,
            after_market_order: false,
            amo_time: "OPEN".to_string(),
            bo_profit_value: 0.0,
            bo_stop_loss_value: 0.0,
        };

        // Place-and-return: Dhan responds as soon as the order is accepted,
        // which may be a non-terminal status (TRANSIT/PENDING). We return the
        // status as-is; `DhanBroker::execute` decides how to treat each state
        // (fill vs. submitted vs. rejected). No retry — see
        // `post_order_no_retry`.
        let response = self.post_order_no_retry(&body, &correlation_id).await?;
        let status = Self::order_status(&response.order_status);

        Ok(OrderResult {
            order_id: response.order_id,
            status,
            timestamp: Utc::now().timestamp_millis(),
            correlation_id,
        })
    }

    async fn get_positions(&self) -> Result<Vec<Position>> {
        let positions = self.get::<Vec<DhanPosition>>("/positions").await?;
        Ok(positions
            .into_iter()
            .filter(|position| position.net_qty != 0)
            .map(|position| Position {
                symbol: position.trading_symbol,
                quantity: position.net_qty,
                average_price: position.buy_avg,
                ltp: position.buy_avg,
                realized_pnl: position.realized_profit,
                unrealized_pnl: position.unrealized_profit,
            })
            .collect())
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
    fn maps_dhan_order_status_strings() {
        assert_eq!(DhanClient::order_status("TRANSIT"), OrderStatus::Transit);
        assert_eq!(DhanClient::order_status("pending"), OrderStatus::Pending);
        assert_eq!(DhanClient::order_status("TRADED"), OrderStatus::Traded);
        assert_eq!(DhanClient::order_status("REJECTED"), OrderStatus::Rejected);
        assert_eq!(DhanClient::order_status("CANCELLED"), OrderStatus::Cancelled);
        assert_eq!(DhanClient::order_status("EXPIRED"), OrderStatus::Expired);
        assert_eq!(
            DhanClient::order_status("SOMETHING_ELSE"),
            OrderStatus::Unknown("SOMETHING_ELSE".to_string())
        );
    }

    #[test]
    fn test_correlation_id_is_present() {
        // The order request body must carry a `correlationId` beginning with
        // "algomln-" so a retried request can be deduplicated by the broker.
        let correlation_id = format!("algomln-{}", uuid::Uuid::new_v4());
        let body = PlaceOrderRequest {
            dhan_client_id: "client".to_string(),
            correlation_id: correlation_id.clone(),
            transaction_type: "BUY".to_string(),
            exchange_segment: "NSE_EQ".to_string(),
            product_type: "INTRADAY".to_string(),
            order_type: "MARKET".to_string(),
            validity: "DAY".to_string(),
            security_id: "2885".to_string(),
            quantity: 1,
            price: 0.0,
            trigger_price: 0.0,
            disclosed_quantity: 0,
            after_market_order: false,
            amo_time: "OPEN".to_string(),
            bo_profit_value: 0.0,
            bo_stop_loss_value: 0.0,
        };

        let json: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&body).expect("serialize place order request"),
        )
        .expect("valid json");

        let correlation = json
            .get("correlationId")
            .and_then(|v| v.as_str())
            .expect("correlationId field present");
        assert!(
            correlation.starts_with("algomln-"),
            "correlationId was {correlation}"
        );
    }

    #[tokio::test]
    async fn test_place_order_rejects_non_equity() {
        use crate::models::{Order, OrderSide, OrderType};

        let mut symbol_map = SymbolMap::empty();
        symbol_map.insert_entry("BANKNIFTY", 1234, Segment::NseFno);
        let symbol_map = Arc::new(parking_lot::RwLock::new(symbol_map));

        let auth = DhanAuth::with_client_id("test-token", "client-1").unwrap();
        let client = DhanClient::with_symbol_map(auth, symbol_map);

        let order = Order {
            symbol: "BANKNIFTY".to_string(),
            side: OrderSide::Buy,
            quantity: 1,
            order_type: OrderType::Market,
            price: None,
        };

        // The segment guard runs before any HTTP request, so this fails fast
        // without touching the network.
        let error = client
            .place_order(order)
            .await
            .expect_err("non-equity order must be rejected");
        assert!(
            error.to_string().contains("Phase 7 only supports NSE equity"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn converts_dhan_unix_seconds_to_unix_ms() {
        assert_eq!(
            DhanClient::dhan_timestamp_to_unix_ms(1_779_820_200.0),
            Some(1_779_820_200_000)
        );
    }

    #[test]
    fn converts_dhan_historical_seconds_to_unix_ms() {
        assert_eq!(
            DhanClient::dhan_historical_timestamp_to_unix_ms(0.0),
            Some(315_532_800_000)
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
