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
pub struct HistoricalRequest<'a> {
    pub security_id: &'a str,
    pub exchange_segment: &'a str,
    pub instrument: &'a str,
    pub expiry_code: i32,
    pub from_date: String,
    pub to_date: String,
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
