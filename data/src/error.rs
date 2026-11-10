use dhan_api::ApiError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Api(#[from] ApiError),
    #[error("invalid data: {0}")]
    Invalid(String),
    #[error("uncovered range for {0}; run `algomln data update` first")]
    Uncovered(String),
    #[error("warmup short for {symbol}: have {have} bars, need {need}")]
    WarmupShort {
        symbol: String,
        have: usize,
        need: usize,
    },
    #[error("daily request budget reached ({used}/{cap})")]
    BudgetExhausted { used: u64, cap: u64 },
}

impl DataError {
    /// True when the run must abort immediately (bad credentials). Everything
    /// else that reaches the bulk layer is recorded as a failure and skipped.
    pub fn is_auth(&self) -> bool {
        matches!(self, DataError::Api(ApiError::Auth(_)))
    }
    /// True when the daily request budget has been reached.
    pub fn is_budget(&self) -> bool {
        matches!(self, DataError::BudgetExhausted { .. })
    }
    /// A short, stable code suitable for the `failures.error_code` column.
    pub fn code(&self) -> String {
        match self {
            DataError::Api(ApiError::Auth(_)) => "auth".into(),
            DataError::Api(ApiError::RateLimit { code, .. }) => code.clone(),
            DataError::Api(ApiError::DataApi { code, .. }) => code.clone(),
            DataError::Api(ApiError::BadRequest { code, .. }) => code.clone(),
            DataError::Api(ApiError::SevereRateLimit(_)) => "DH-805".into(),
            DataError::Api(ApiError::DataUnavailable(_)) => "DH-907".into(),
            DataError::Api(ApiError::Access(_)) => "access".into(),
            DataError::Api(ApiError::Server(_)) => "server".into(),
            DataError::Api(ApiError::Transport(_)) => "transport".into(),
            DataError::Api(ApiError::Malformed(_)) => "malformed".into(),
            DataError::BudgetExhausted { .. } => "budget".into(),
            DataError::Io(_) => "io".into(),
            DataError::Db(_) => "db".into(),
            DataError::Invalid(_) => "invalid".into(),
            DataError::Uncovered(_) => "uncovered".into(),
            DataError::WarmupShort { .. } => "warmup_short".into(),
        }
    }
}
