pub mod dhan;

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
    M30,
    H1,
    H4,
    D1,
    W1,
}

impl Timeframe {
    pub fn as_dhan_interval(self) -> &'static str {
        match self {
            Self::M1 => "1",
            Self::M5 => "5",
            Self::M15 => "15",
            Self::M30 => "30",
            Self::H1 => "60",
            Self::H4 => "240",
            Self::D1 => "D",
            Self::W1 => "W",
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
