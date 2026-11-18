use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_INSTRUMENTS_PER_MESSAGE: usize = 100;
pub const MAX_INSTRUMENTS_PER_CONNECTION: usize = 5000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestCode {
    Ticker = 15,
    Quote = 17,
    Full = 21,
    Disconnect = 12,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstrumentSubscription {
    #[serde(rename = "ExchangeSegment")]
    pub exchange_segment: String,
    #[serde(rename = "SecurityId")]
    pub security_id: String,
}

impl InstrumentSubscription {
    pub fn nse_eq(security_id: impl Into<String>) -> Self {
        Self {
            exchange_segment: "NSE_EQ".to_string(),
            security_id: security_id.into(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SubscribeMessage {
    #[serde(rename = "RequestCode")]
    pub request_code: u8,
    #[serde(rename = "InstrumentCount")]
    pub instrument_count: usize,
    #[serde(rename = "InstrumentList")]
    pub instrument_list: Vec<InstrumentSubscription>,
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum SubscriptionError {
    #[error("instruments count {count} exceeds maximum connection limit of {max}", max = MAX_INSTRUMENTS_PER_CONNECTION)]
    ExceedsConnectionLimit { count: usize },
    #[error("empty instruments list")]
    EmptyList,
}

/// Batches an instrument list into `SubscribeMessage`s of up to 100 instruments each.
/// Enforces the 5,000 instrument per-connection limit.
pub fn batch_subscriptions(
    code: RequestCode,
    instruments: &[InstrumentSubscription],
) -> Result<Vec<SubscribeMessage>, SubscriptionError> {
    if instruments.is_empty() {
        return Err(SubscriptionError::EmptyList);
    }
    if instruments.len() > MAX_INSTRUMENTS_PER_CONNECTION {
        return Err(SubscriptionError::ExceedsConnectionLimit {
            count: instruments.len(),
        });
    }

    let mut messages = Vec::new();
    for chunk in instruments.chunks(MAX_INSTRUMENTS_PER_MESSAGE) {
        messages.push(SubscribeMessage {
            request_code: code as u8,
            instrument_count: chunk.len(),
            instrument_list: chunk.to_vec(),
        });
    }

    Ok(messages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_batch_single_chunk() {
        let instruments = vec![
            InstrumentSubscription::nse_eq("2885"),
            InstrumentSubscription::nse_eq("1333"),
        ];
        let batches = batch_subscriptions(RequestCode::Ticker, &instruments).unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].request_code, 15);
        assert_eq!(batches[0].instrument_count, 2);
        assert_eq!(batches[0].instrument_list.len(), 2);
    }

    #[test]
    fn test_batch_multiple_chunks() {
        let instruments: Vec<InstrumentSubscription> = (1..=250)
            .map(|i| InstrumentSubscription::nse_eq(i.to_string()))
            .collect();

        let batches = batch_subscriptions(RequestCode::Ticker, &instruments).unwrap();
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].instrument_count, 100);
        assert_eq!(batches[1].instrument_count, 100);
        assert_eq!(batches[2].instrument_count, 50);

        let json = serde_json::to_string(&batches[0]).unwrap();
        assert!(json.contains(r#""RequestCode":15"#));
        assert!(json.contains(r#""InstrumentCount":100"#));
        assert!(json.contains(r#""InstrumentList":[{"ExchangeSegment":"NSE_EQ""#));
    }

    #[test]
    fn test_batch_limits() {
        assert_eq!(
            batch_subscriptions(RequestCode::Ticker, &[]).unwrap_err(),
            SubscriptionError::EmptyList
        );

        let too_many: Vec<InstrumentSubscription> = (0..5001)
            .map(|i| InstrumentSubscription::nse_eq(i.to_string()))
            .collect();
        assert_eq!(
            batch_subscriptions(RequestCode::Ticker, &too_many).unwrap_err(),
            SubscriptionError::ExceedsConnectionLimit { count: 5001 }
        );
    }
}
