use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    engine::Symbol,
    trader::{OrderId, OrderReason, Side, TraderEvent},
};
use super::source::{SourceError, SourceStats};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Timeframe {
    M1,
    M5,
    M15,
    M25,
    M60,
    D1,
}

impl Timeframe {
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
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateRange {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint(pub String);

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GapPolicy {
    None,
    FillFlatInSession,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Broker fees are accrued and rounded in integer paise using ties-to-even.
pub struct FeeModel {
    pub bps: f64,
    pub flat_per_order: f64,
    pub min_per_order: f64,
}

pub(crate) fn round_half_even_i128(numerator: i128, denominator: i128) -> i128 {
    let q = numerator / denominator;
    let r = numerator % denominator;
    let twice = r.abs() * 2;
    if twice > denominator.abs() || (twice == denominator.abs() && q % 2 != 0) {
        q + numerator.signum() * denominator.signum()
    } else {
        q
    }
}

pub(crate) fn round_half_even_f64(value: f64) -> f64 {
    let floor = value.floor();
    let fraction = value - floor;
    if fraction < 0.5 {
        floor
    } else if fraction > 0.5 || (floor as i128).rem_euclid(2) != 0 {
        floor + 1.0
    } else {
        floor
    }
}

impl Default for FeeModel {
    fn default() -> Self {
        Self {
            bps: 0.0,
            flat_per_order: 0.0,
            min_per_order: 0.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunConfig {
    pub timeframe: Timeframe,
    pub range: DateRange,
    pub initial_cash: f64,
    pub warmup_bars: usize,
    pub latency: u64,
    pub slippage_bps: f64,
    pub fee_model: FeeModel,
    pub partial_fills_pct: Option<f64>,
    pub gap_policy: GapPolicy,
    pub debug_invariants: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub ts: i64,
    pub symbol: Symbol,
    pub order_id: OrderId,
    pub side: Side,
    pub qty: u64,
    pub price: f64,
    pub price_paise: i64,
    pub fees: f64,
    pub fees_paise: i64,
    pub reason: OrderReason,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EquityPoint {
    pub ts: i64,
    pub cash: f64,
    pub positions_value: f64,
    pub equity: f64,
    pub cash_paise: i64,
    pub positions_value_paise: i64,
    pub equity_paise: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenPosition {
    pub symbol: Symbol,
    pub qty: i64,
    pub avg_entry: f64,
    pub last_close: f64,
    pub unrealized: f64,
    pub avg_entry_paise: i64,
    pub last_close_paise: i64,
    pub unrealized_paise: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    pub symbol: Symbol,
    pub entry_ts: i64,
    pub exit_ts: i64,
    pub qty: u64,
    pub avg_entry: f64,
    pub avg_entry_paise: i64,
    pub avg_exit: f64,
    pub avg_exit_paise: i64,
    pub fees: f64,
    pub fees_paise: i64,
    pub net_pnl: f64,
    pub net_pnl_paise: i64,
    pub exit_reason: OrderReason,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolEquityPoint {
    pub ts: i64,
    pub equity: f64,
    pub equity_paise: i64,
    pub position_qty: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct StageTimings {
    pub load_merge: std::time::Duration,
    pub engine_indicators: std::time::Duration,
    pub trader: std::time::Duration,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunResult {
    pub fills: Vec<Fill>,
    pub equity: Vec<EquityPoint>,
    pub events: Vec<TraderEvent>,
    pub open_positions: Vec<OpenPosition>,
    pub manifest: serde_json::Value,
    pub source_stats: SourceStats,
    pub trades: Vec<Trade>,
    pub symbol_equity: BTreeMap<Symbol, Vec<SymbolEquityPoint>>,
    pub timings: StageTimings,
}

#[derive(Debug, Error)]
pub enum SimError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Engine(#[from] crate::engine::EngineError),
    #[error("{0}")]
    Other(String),
}
