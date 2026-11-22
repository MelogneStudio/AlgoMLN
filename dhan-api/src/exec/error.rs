use serde::{Deserialize, Serialize};

/// Error classification as defined in Section 8 of the Dhan ExecutionPort specification.
/// Higher rules: a rejection is never retried automatically, and an unknown outcome
/// never becomes a second order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorClass {
    /// HTTP 401, DH-901: Lock entries, alert. Try RenewToken once if the token is still valid.
    FatalAuth,
    /// DH-902: Lock entries, alert (segment access, or live static IP missing).
    FatalConfig,
    /// DH-903: Lock entries, alert.
    FatalAccount,
    /// DH-904, RL001 (RATE_LIMIT_ERROR), HTTP 429: Back off at least 1s, requeue, exits first,
    /// confirm by correlation lookup before resending.
    Throttle,
    /// DH-905: Our bug. Never retry, halt that symbol, log the full payload.
    OurBug,
    /// DH-906: Business rejection. No retry, emit Rejected, read the message for the cause.
    BusinessRejection,
    /// DH-907: No data or bad parameters. Treat as rejection, keep the raw payload.
    BadRequest,
    /// DH-908: Transient server error. Look up by correlation id first, retry with backoff only if no order exists.
    TransientServerError,
    /// DH-909, timeout, 5xx, connection reset: Unknown outcome. Look up by correlation id, never blind retry.
    UnknownOutcome,
    /// DH-910, anything unseen: Unclassified. Treat as rejection, alert.
    Unclassified,
}

impl ErrorClass {
    pub fn locks_entries(&self) -> bool {
        matches!(
            self,
            ErrorClass::FatalAuth
                | ErrorClass::FatalConfig
                | ErrorClass::FatalAccount
                | ErrorClass::OurBug
        )
    }

    pub fn is_throttle(&self) -> bool {
        matches!(self, ErrorClass::Throttle)
    }

    pub fn is_unknown_outcome(&self) -> bool {
        matches!(self, ErrorClass::UnknownOutcome | ErrorClass::TransientServerError)
    }
}

/// Classify an API / network / OMS failure based on HTTP status code,
/// error code string, error type, and error message.
pub fn classify_error(
    status_code: Option<u16>,
    error_code: Option<&str>,
    error_type: Option<&str>,
    _message: Option<&str>,
) -> ErrorClass {
    // Check specific HTTP status codes first if present
    if let Some(status) = status_code {
        if status == 401 {
            return ErrorClass::FatalAuth;
        }
        if status == 429 {
            return ErrorClass::Throttle;
        }
    }

    let code_upper = error_code.map(|c| c.to_ascii_uppercase());
    let type_upper = error_type.map(|t| t.to_ascii_uppercase());

    if let Some(ref t) = type_upper {
        if t == "RATE_LIMIT_ERROR" {
            return ErrorClass::Throttle;
        }
    }

    if let Some(ref c) = code_upper {
        match c.as_str() {
            "DH-901" => return ErrorClass::FatalAuth,
            "DH-902" => return ErrorClass::FatalConfig,
            "DH-903" => return ErrorClass::FatalAccount,
            "DH-904" | "RL001" => return ErrorClass::Throttle,
            "DH-905" => return ErrorClass::OurBug,
            "DH-906" => return ErrorClass::BusinessRejection,
            "DH-907" => return ErrorClass::BadRequest,
            "DH-908" => return ErrorClass::TransientServerError,
            "DH-909" => return ErrorClass::UnknownOutcome,
            "DH-910" => return ErrorClass::Unclassified,
            _ => {}
        }
    }

    // Check 5xx or server transport errors
    if let Some(status) = status_code {
        if (500..=599).contains(&status) {
            return ErrorClass::UnknownOutcome;
        }
    }

    ErrorClass::Unclassified
}
