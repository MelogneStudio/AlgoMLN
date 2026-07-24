use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhanSymbol {
    pub security_id: String,
    pub exchange_segment: String,
    pub instrument: String,
}

impl DhanSymbol {
    pub fn parse(input: &str, default_exchange_segment: &str, default_instrument: &str) -> Self {
        let parts = input
            .split(['|', ':'])
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();

        Self {
            security_id: parts.first().copied().unwrap_or(input).to_string(),
            exchange_segment: parts
                .get(1)
                .copied()
                .unwrap_or(default_exchange_segment)
                .to_string(),
            instrument: parts
                .get(2)
                .copied()
                .unwrap_or(default_instrument)
                .to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IntradayRequest {
    pub security_id: String,
    pub exchange_segment: String,
    pub instrument: String,
    pub interval: String,
    pub oi: bool,
    pub from_date: String,
    pub to_date: String,
}

#[derive(Debug, Deserialize)]
pub struct HistoricalResponse {
    pub timestamp: Vec<Option<f64>>,
    pub open: Vec<Option<f64>>,
    pub high: Vec<Option<f64>>,
    pub low: Vec<Option<f64>>,
    pub close: Vec<Option<f64>>,
    pub volume: Vec<Option<f64>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DhanQuoteValue {
    pub last_price: Option<f64>,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: Option<f64>,
    pub volume: Option<u64>,
    pub bid_price: Option<f64>,
    pub ask_price: Option<f64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceOrderRequest {
    pub dhan_client_id: String,
    #[serde(rename = "transactionType")]
    pub transaction_type: String,
    pub exchange_segment: String,
    pub product_type: String,
    pub order_type: String,
    pub validity: String,
    pub security_id: String,
    pub quantity: u32,
    pub price: f64,
    pub trigger_price: f64,
    pub disclosed_quantity: u32,
    pub after_market_order: bool,
    pub amo_time: String,
    pub bo_profit_value: f64,
    pub bo_stop_loss_value: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaceOrderResponse {
    pub order_id: String,
    pub order_status: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DhanPosition {
    pub security_id: String,
    pub trading_symbol: String,
    pub exchange_segment: String,
    pub product_type: String,
    pub buy_avg: f64,
    pub buy_qty: i64,
    pub sell_avg: f64,
    pub sell_qty: i64,
    pub net_qty: i64,
    pub realized_profit: f64,
    pub unrealized_profit: f64,
    pub day_buy_value: f64,
    pub day_sell_value: f64,
}
