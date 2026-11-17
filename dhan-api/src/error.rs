use serde::Deserialize;
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum ApiError {
    #[error("authentication failed ({0}); refresh the token")]
    Auth(String),
    #[error("rate limited ({code}): {message}")]
    RateLimit {
        code: String,
        message: String,
        retry_after: Option<Duration>,
    },
    #[error("Dhan data API error ({code}): {message}")]
    DataApi { code: String, message: String },
    #[error("server error: {0}")]
    Server(String),
    #[error("malformed response: {0}")]
    Malformed(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("severe rate limit (DH-805): {0}")]
    SevereRateLimit(String),
    #[error("Dhan access denied ({0})")]
    Access(String),
    #[error("Dhan bad request ({code}): {message}")]
    BadRequest { code: String, message: String },
    #[error("data unavailable ({0})")]
    DataUnavailable(String),
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiErrorBody {
    #[serde(default, rename = "errorCode")]
    pub(crate) error_code: String,
    #[serde(default, rename = "errorMessage")]
    pub(crate) error_message: String,
    #[serde(default)]
    pub(crate) status: Option<String>,
    #[serde(default)]
    pub(crate) remarks: Option<String>,
}
