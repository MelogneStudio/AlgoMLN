use serde::{Deserialize, Serialize};

/// Identifies an order inside AlgoMLN before and during broker submission.
pub type LocalOrderId = String;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Side::Buy => write!(f, "BUY"),
            Side::Sell => write!(f, "SELL"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderKind {
    Market,
    Limit { price_paise: i64 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderIntent {
    Entry,
    Exit,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderRequest {
    pub local_id: LocalOrderId,
    pub security_id: String,
    pub side: Side,
    pub quantity: u64,
    pub kind: OrderKind,
    pub intent: OrderIntent,
    pub local_seq: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecEvent {
    Submitted {
        local_id: LocalOrderId,
        dhan_order_id: Option<String>,
    },
    Open {
        local_id: LocalOrderId,
        dhan_order_id: String,
    },
    Fill {
        local_id: LocalOrderId,
        qty: u64,
        price_paise: i64,
        exchange_trade_id: String,
    },
    Filled {
        local_id: LocalOrderId,
    },
    Rejected {
        local_id: LocalOrderId,
        reason: String,
        error_class: crate::exec::error::ErrorClass,
    },
    Cancelled {
        local_id: LocalOrderId,
        remaining_qty: u64,
    },
    Expired {
        local_id: LocalOrderId,
        remaining_qty: u64,
    },
    Unknown {
        local_id: LocalOrderId,
        reason: String,
    },
    ExternalChange {
        security_id: String,
        net_qty: i64,
    },
    Halt {
        reason: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ExecutionMode {
    #[default]
    Dry,
    Sandbox,
    Live,
}

impl std::str::FromStr for ExecutionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "dry" => Ok(ExecutionMode::Dry),
            "sandbox" => Ok(ExecutionMode::Sandbox),
            "live" => Ok(ExecutionMode::Live),
            other => Err(format!(
                "unknown execution mode '{other}'; valid modes are: dry, sandbox, live"
            )),
        }
    }
}

impl std::fmt::Display for ExecutionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecutionMode::Dry => write!(f, "dry"),
            ExecutionMode::Sandbox => write!(f, "sandbox"),
            ExecutionMode::Live => write!(f, "live"),
        }
    }
}

pub const LIVE_BASE_URL: &str = "https://api.dhan.co/v2";
pub const SANDBOX_BASE_URL: &str = "https://sandbox.dhan.co/v2";
pub const LIVE_ORDER_UPDATE_WS: &str = "wss://api-order-update.dhan.co";

#[derive(Clone, Debug)]
pub struct ModeConfig {
    pub mode: ExecutionMode,
    pub base_url: String,
    pub token: Option<String>,
    pub live_trading_acknowledged: bool,
}

impl ModeConfig {
    /// Resolve configuration strictly according to Section 2 rules.
    /// - Base URL is derived directly from the mode (or test override).
    /// - Each mode checks its dedicated environment variable:
    ///   - Sandbox: DHAN_SANDBOX_ACCESS_TOKEN
    ///   - Live: DHAN_ACCESS_TOKEN
    /// - Missing token refuses to start. Never falls back to the other token.
    /// - Live mode requires live_flag (from --exec live) and LIVE_TRADING=true.
    pub fn resolve(
        mode: ExecutionMode,
        test_base_url_override: Option<String>,
    ) -> Result<Self, String> {
        match mode {
            ExecutionMode::Dry => {
                let base_url = test_base_url_override.unwrap_or_else(|| "dry://local".into());
                Ok(Self {
                    mode: ExecutionMode::Dry,
                    base_url,
                    token: None,
                    live_trading_acknowledged: false,
                })
            }
            ExecutionMode::Sandbox => {
                let token = std::env::var("DHAN_SANDBOX_ACCESS_TOKEN")
                    .map(|s| s.trim().to_string())
                    .map_err(|_| {
                        "sandbox mode requires DHAN_SANDBOX_ACCESS_TOKEN environment variable; refusing to start".to_string()
                    })?;
                if token.is_empty() {
                    return Err(
                        "DHAN_SANDBOX_ACCESS_TOKEN is empty; refusing to start".to_string()
                    );
                }
                let base_url = test_base_url_override
                    .unwrap_or_else(|| SANDBOX_BASE_URL.to_string());
                Ok(Self {
                    mode: ExecutionMode::Sandbox,
                    base_url,
                    token: Some(token),
                    live_trading_acknowledged: false,
                })
            }
            ExecutionMode::Live => {
                let live_env = std::env::var("LIVE_TRADING")
                    .unwrap_or_default()
                    .trim()
                    .eq_ignore_ascii_case("true");
                if !live_env {
                    return Err(
                        "live execution mode requires LIVE_TRADING=true environment variable; refusing to start"
                            .to_string(),
                    );
                }
                let token = std::env::var("DHAN_ACCESS_TOKEN")
                    .map(|s| s.trim().to_string())
                    .map_err(|_| {
                        "live mode requires DHAN_ACCESS_TOKEN environment variable; refusing to start".to_string()
                    })?;
                if token.is_empty() {
                    return Err("DHAN_ACCESS_TOKEN is empty; refusing to start".to_string());
                }
                let base_url = test_base_url_override
                    .unwrap_or_else(|| LIVE_BASE_URL.to_string());
                Ok(Self {
                    mode: ExecutionMode::Live,
                    base_url,
                    token: Some(token),
                    live_trading_acknowledged: true,
                })
            }
        }
    }
}
