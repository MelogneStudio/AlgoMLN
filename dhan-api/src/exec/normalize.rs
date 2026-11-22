use crate::exec::types::Side;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdateSource {
    Rest,
    Websocket,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NormalizedStatus {
    Submitted,
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Expired,
    ClosedLogOnly,
    Unknown(String),
}

/// Normalized order update merging REST and WebSocket fields per Section 6.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedOrderUpdate {
    pub source: UpdateSource,
    pub dhan_order_id: Option<String>,
    pub correlation_id: Option<String>,
    pub status: NormalizedStatus,
    pub quantity: u64,
    pub cum_traded_qty: u64,
    pub remaining_qty: u64,
    pub avg_price_paise: Option<i64>,
    pub last_traded_price_paise: Option<i64>,
    pub side: Option<Side>,
    pub reject_reason: Option<String>,
    pub update_time: Option<String>,
}

/// Convert a float or decimal rupee string/number to integer paise rounding half-up.
pub fn rupees_to_paise_half_up(val: f64) -> i64 {
    if !val.is_finite() || val <= 0.0 {
        return 0;
    }
    // Round half-up: add 0.5 to paise then floor
    (val * 100.0 + 0.5).floor() as i64
}

/// Parse rupee price from serde_json::Value (handles integer, float, string)
pub fn parse_rupees_to_paise(val: &serde_json::Value) -> Option<i64> {
    if let Some(s) = val.as_str() {
        let s = s.trim();
        if let Some((int_part, frac_part)) = s.split_once('.') {
            let whole: i64 = int_part.parse().ok()?;
            let mut frac_str = frac_part.to_string();
            if frac_str.len() == 1 {
                frac_str.push('0');
            } else if frac_str.len() > 2 {
                let round_up = frac_str.as_bytes()[2] >= b'5';
                frac_str.truncate(2);
                let mut paise: i64 = frac_str.parse().ok()?;
                if round_up {
                    paise += 1;
                }
                return Some(whole * 100 + paise);
            }
            let paise: i64 = frac_str.parse().ok()?;
            return Some(whole * 100 + paise);
        } else if let Ok(whole) = s.parse::<i64>() {
            return Some(whole * 100);
        }
    }
    if let Some(f) = val.as_f64() {
        Some(rupees_to_paise_half_up(f))
    } else if let Some(i) = val.as_i64() {
        Some(i * 100)
    } else {
        None
    }
}

/// Match statuses case-insensitively according to Section 4 mapping:
/// - TRANSIT -> Submitted
/// - PENDING -> Open, or PartiallyFilled if traded qty > 0 and remaining > 0, or Filled if traded > 0 and remaining == 0
/// - PART_TRADED -> PartiallyFilled
/// - TRADED -> Filled
/// - CANCELLED -> Cancelled
/// - REJECTED -> Rejected
/// - EXPIRED -> Expired
/// - MODIFIED, TRIGGERED -> Open (not expected in v1)
/// - CLOSED -> ClosedLogOnly (super orders)
/// - anything else -> Unknown(status)
pub fn map_status(status_raw: &str, traded_qty: u64, remaining_qty: u64) -> NormalizedStatus {
    let s = status_raw.trim().to_ascii_uppercase();
    match s.as_str() {
        "TRANSIT" => NormalizedStatus::Submitted,
        "PENDING" => {
            if traded_qty > 0 && remaining_qty > 0 {
                NormalizedStatus::PartiallyFilled
            } else if traded_qty > 0 && remaining_qty == 0 {
                NormalizedStatus::Filled
            } else {
                NormalizedStatus::Open
            }
        }
        "PART_TRADED" | "PART-TRADED" | "PARTIALLY_TRADED" | "PARTIAL_TRADED" => {
            NormalizedStatus::PartiallyFilled
        }
        "TRADED" | "FILLED" => NormalizedStatus::Filled,
        "CANCELLED" | "CANCELED" => NormalizedStatus::Cancelled,
        "REJECTED" => NormalizedStatus::Rejected,
        "EXPIRED" => NormalizedStatus::Expired,
        "MODIFIED" | "TRIGGERED" => NormalizedStatus::Open,
        "CLOSED" => NormalizedStatus::ClosedLogOnly,
        _ => NormalizedStatus::Unknown(status_raw.to_string()),
    }
}

pub fn parse_side(val: &serde_json::Value) -> Option<Side> {
    let s = val.as_str()?.trim().to_ascii_uppercase();
    match s.as_str() {
        "BUY" | "B" => Some(Side::Buy),
        "SELL" | "S" => Some(Side::Sell),
        _ => None,
    }
}

pub fn parse_u64_field(val: &serde_json::Value) -> u64 {
    if let Some(n) = val.as_u64() {
        n
    } else if let Some(i) = val.as_i64() {
        i.max(0) as u64
    } else if let Some(f) = val.as_f64() {
        f.max(0.0).floor() as u64
    } else if let Some(s) = val.as_str() {
        s.trim().parse::<f64>().unwrap_or(0.0).max(0.0).floor() as u64
    } else {
        0
    }
}

/// Parse a REST order object (from GET /orders or GET /orders/{id} or POST /orders response).
pub fn parse_rest_order(val: &serde_json::Value) -> Result<NormalizedOrderUpdate, String> {
    let obj = val
        .as_object()
        .ok_or_else(|| "REST order payload must be a JSON object".to_string())?;

    let dhan_order_id = obj
        .get("orderId")
        .and_then(|v| v.as_str().or_else(|| v.as_i64().map(|_| "")))
        .map(|s| {
            if s.is_empty() {
                obj.get("orderId").unwrap().to_string()
            } else {
                s.to_string()
            }
        });

    let correlation_id = obj
        .get("correlationId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let quantity = obj.get("quantity").map(parse_u64_field).unwrap_or(0);
    let cum_traded_qty = obj.get("filledQty").map(parse_u64_field).unwrap_or(0);
    let remaining_qty = obj
        .get("remainingQuantity")
        .map(parse_u64_field)
        .unwrap_or_else(|| quantity.saturating_sub(cum_traded_qty));

    let status_raw = obj
        .get("orderStatus")
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN");
    let status = map_status(status_raw, cum_traded_qty, remaining_qty);

    let avg_price_paise = obj.get("averageTradedPrice").and_then(parse_rupees_to_paise);
    let side = obj.get("transactionType").and_then(parse_side);

    let reject_reason = obj
        .get("omsErrorDescription")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            obj.get("omsErrorCode")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from)
        });

    let update_time = obj
        .get("updateTime")
        .or_else(|| obj.get("createTime"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id,
        correlation_id,
        status,
        quantity,
        cum_traded_qty,
        remaining_qty,
        avg_price_paise,
        last_traded_price_paise: None,
        side,
        reject_reason,
        update_time,
    })
}

/// Parse a WebSocket order update frame (Data object or wrapper).
pub fn parse_ws_order(val: &serde_json::Value) -> Result<NormalizedOrderUpdate, String> {
    // Frame may be {"Type":"order_alert","Data":{...}} or directly the Data object
    let obj = if let Some(data) = val.get("Data").and_then(|d| d.as_object()) {
        data
    } else if let Some(o) = val.as_object() {
        o
    } else {
        return Err("WebSocket order payload must be a JSON object".to_string());
    };

    let dhan_order_id = obj
        .get("OrderNo")
        .or_else(|| obj.get("orderNo"))
        .or_else(|| obj.get("orderId"))
        .and_then(|v| v.as_str().or_else(|| v.as_i64().map(|_| "")))
        .map(|s| {
            if s.is_empty() {
                val.to_string()
            } else {
                s.to_string()
            }
        });

    let correlation_id = obj
        .get("CorrelationId")
        .or_else(|| obj.get("correlationId"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let quantity = obj
        .get("Quantity")
        .or_else(|| obj.get("quantity"))
        .map(parse_u64_field)
        .unwrap_or(0);
    let cum_traded_qty = obj
        .get("TradedQty")
        .or_else(|| obj.get("tradedQty"))
        .or_else(|| obj.get("filledQty"))
        .map(parse_u64_field)
        .unwrap_or(0);
    let remaining_qty = obj
        .get("RemainingQuantity")
        .or_else(|| obj.get("remainingQuantity"))
        .map(parse_u64_field)
        .unwrap_or_else(|| quantity.saturating_sub(cum_traded_qty));

    let status_raw = obj
        .get("Status")
        .or_else(|| obj.get("status"))
        .or_else(|| obj.get("orderStatus"))
        .and_then(|v| v.as_str())
        .unwrap_or("UNKNOWN");
    let status = map_status(status_raw, cum_traded_qty, remaining_qty);

    let avg_price_paise = obj
        .get("AvgTradedPrice")
        .or_else(|| obj.get("avgTradedPrice"))
        .or_else(|| obj.get("averageTradedPrice"))
        .and_then(parse_rupees_to_paise);

    let last_traded_price_paise = obj
        .get("TradedPrice")
        .or_else(|| obj.get("tradedPrice"))
        .and_then(parse_rupees_to_paise);

    let side = obj
        .get("TransactionType")
        .or_else(|| obj.get("transactionType"))
        .and_then(parse_side);

    let reject_reason = obj
        .get("ReasonDescription")
        .or_else(|| obj.get("reasonDescription"))
        .or_else(|| obj.get("omsErrorDescription"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);

    let update_time = obj
        .get("LastUpdatedTime")
        .or_else(|| obj.get("lastUpdatedTime"))
        .or_else(|| obj.get("updateTime"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Ok(NormalizedOrderUpdate {
        source: UpdateSource::Websocket,
        dhan_order_id,
        correlation_id,
        status,
        quantity,
        cum_traded_qty,
        remaining_qty,
        avg_price_paise,
        last_traded_price_paise,
        side,
        reject_reason,
        update_time,
    })
}
