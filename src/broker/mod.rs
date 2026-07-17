pub mod dhan;
pub mod symbol_map;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::models::{Candle, Order, OrderResult, Portfolio, Position, Quote};

pub type BrokerResult<T> = anyhow::Result<T>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Timeframe {
    M1,
    M5,
    M15,
    M25,
    M30,
    M60,
    H1,
    H4,
    D1,
    W1,
}

impl Timeframe {
    pub fn is_intraday(&self) -> bool {
        matches!(
            self,
            Timeframe::M1 | Timeframe::M5 | Timeframe::M15 | Timeframe::M25 | Timeframe::M60
        )
    }

    pub fn to_interval_str(&self) -> &str {
        match self {
            Timeframe::M1 => "1",
            Timeframe::M5 => "5",
            Timeframe::M15 => "15",
            Timeframe::M25 => "25",
            Timeframe::M60 => "60",
            _ => "1",
        }
    }

    pub fn as_dhan_interval(self) -> &'static str {
        match self {
            Self::M1 => "1",
            Self::M5 => "5",
            Self::M15 => "15",
            Self::M25 => "25",
            Self::M30 => "30",
            Self::M60 => "60",
            Self::H1 => "60",
            Self::H4 => "240",
            Self::D1 => "D",
            Self::W1 => "W",
        }
    }
}

impl std::str::FromStr for Timeframe {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_uppercase().as_str() {
            "M1" | "1" => Ok(Self::M1),
            "M5" | "5" => Ok(Self::M5),
            "M15" | "15" => Ok(Self::M15),
            "M25" | "25" => Ok(Self::M25),
            "M30" | "30" => Ok(Self::M30),
            "M60" | "60" | "H1" => Ok(Self::M60),
            "H4" => Ok(Self::H4),
            "D1" | "1D" | "D" => Ok(Self::D1),
            "W1" | "1W" | "W" => Ok(Self::W1),
            _ => Err(format!("Unsupported timeframe: {value}")),
        }
    }
}

#[async_trait]
pub trait BrokerClient: Send + Sync {
    async fn get_ohlcv(
        &self,
        symbol: &str,
        timeframe: Timeframe,
        from: i64,
        to: i64,
    ) -> BrokerResult<Vec<Candle>>;

    async fn get_quote(&self, symbol: &str) -> BrokerResult<Quote>;

    async fn place_order(&self, order: Order) -> BrokerResult<OrderResult>;

    async fn get_positions(&self) -> BrokerResult<Vec<Position>>;

    async fn get_portfolio(&self) -> BrokerResult<Portfolio>;
}
