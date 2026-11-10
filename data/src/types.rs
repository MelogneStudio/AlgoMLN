use chrono::NaiveDate;
use dhan_api::Candle as DhanCandle;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tf {
    M1,
    M5,
    M15,
    M25,
    M60,
    D1,
}

impl Tf {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::M1 => "1m",
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::M25 => "25m",
            Self::M60 => "60m",
            Self::D1 => "1d",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "1m" => Self::M1,
            "5m" => Self::M5,
            "15m" => Self::M15,
            "25m" => Self::M25,
            "60m" => Self::M60,
            "1d" => Self::D1,
            _ => return None,
        })
    }

    pub fn seconds(self) -> i64 {
        match self {
            Self::M1 => 60,
            Self::M5 => 300,
            Self::M15 => 900,
            Self::M25 => 1500,
            Self::M60 => 3600,
            Self::D1 => 86400,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}

impl From<DhanCandle> for Bar {
    fn from(x: DhanCandle) -> Self {
        Self {
            ts: x.ts,
            open: x.open,
            high: x.high,
            low: x.low,
            close: x.close,
            volume: x.volume,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instrument {
    pub security_id: String,
    pub symbol: String,
    pub segment: String,
    pub lot_size: u32,
    pub tick_size_paise: i64,
    pub refreshed_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quality {
    pub kind: String,
    pub ts: Option<i64>,
    pub detail: String,
}

#[derive(Clone, Debug)]
pub struct PartitionMeta {
    pub path: PathBuf,
    pub checksum: String,
    pub first_ts: i64,
    pub last_ts: i64,
    pub rows: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Coverage {
    pub start_ts: i64,
    pub end_ts: i64,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageRange {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesStatus {
    pub symbol: String,
    pub timeframe: String,
    pub bars: u64,
    pub coverage: Vec<CoverageRange>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualityRow {
    pub symbol: String,
    pub tf: String,
    pub date: String,
    pub kind: String,
}

pub fn validate_bar(b: &Bar) -> bool {
    [b.open, b.high, b.low, b.close, b.volume]
        .iter()
        .all(|x| x.is_finite())
        && b.volume >= 0.0
        && b.low <= b.open.min(b.close)
        && b.high >= b.open.max(b.close)
        && b.ts.rem_euclid(60) == 0
}

pub fn drop_in_progress(bars: Vec<Bar>, now: i64, step: i64) -> Vec<Bar> {
    bars.into_iter().filter(|b| now >= b.ts + step).collect()
}
