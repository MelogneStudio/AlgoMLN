use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tick {
    pub symbol: String,
    pub ltp: f64,
    pub volume: u64,
    pub timestamp: i64,
}
