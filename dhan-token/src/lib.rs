//! # dhan-token
//!
//! Lightweight, standalone crate for extracting and inspecting claims from a Dhan access token (a JWT).
//!
//! ## IMPORTANT: NO SIGNATURE VERIFICATION
//!
//! **DO NOT rely on this crate for cryptographic authentication or signature verification.**
//!
//! This crate decodes and extracts claims solely from the token's payload segment without
//! verifying the cryptographic signature against Dhan's public key or secret. It is intended
//! strictly for client-side claim inspection (such as extracting the client ID, checking expiry,
//! and planning preemptive token renewals before expiration).
//!
//! ## Security & Hygiene
//!
//! - Raw tokens and signature segments are **never** stored inside [`TokenInfo`], logged,
//!   or included in any [`TokenError`] Display or Debug formatting.
//! - Client IDs are considered public for operational logging; tokens and signatures are not.
//! - The token is never written to disk or persisted.

use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::{DateTime, Duration, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Threshold under which a token is considered [`TokenStatus::ExpiringSoon`] (1 hour).
pub const EXPIRING_SOON_THRESHOLD: Duration = Duration::hours(1);

/// Fixed offset for Indian Standard Time (IST, UTC+05:30).
pub fn ist_offset() -> FixedOffset {
    FixedOffset::east_opt(5 * 3600 + 30 * 60).expect("valid IST offset +05:30")
}

/// Token validity status relative to an evaluation time and safety margin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenStatus {
    /// Token is valid and well beyond the expiring-soon threshold.
    Valid { remaining: Duration },
    /// Token is valid but expiring within [`EXPIRING_SOON_THRESHOLD`] (1 hour).
    ExpiringSoon { remaining: Duration },
    /// Token is expired (either past its expiration timestamp or within the specified safety margin).
    Expired { since: Duration },
}

impl std::fmt::Display for TokenStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenStatus::Valid { remaining } => {
                write!(f, "Valid ({}s remaining)", remaining.num_seconds())
            }
            TokenStatus::ExpiringSoon { remaining } => {
                write!(f, "ExpiringSoon ({}s remaining)", remaining.num_seconds())
            }
            TokenStatus::Expired { since } => {
                write!(f, "Expired ({}s ago)", since.num_seconds())
            }
        }
    }
}

/// Errors that can occur when parsing a Dhan access token.
///
/// **Security note:** Error messages and debug output never contain the raw token or signature.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("token is empty")]
    Empty,

    #[error("token does not have exactly 3 segments separated by '.'")]
    NotThreeSegments,

    #[error("failed to base64url-decode token payload: {0}")]
    BadBase64(String),

    #[error("failed to parse token payload JSON: {0}")]
    BadJson(String),

    #[error("token payload missing 'dhanClientId' claim")]
    MissingClientId,

    #[error("token payload missing 'exp' claim")]
    MissingExp,

    #[error("token payload 'exp' claim is not an integer number")]
    ExpNotNumber,
}

impl TokenError {
    /// Stable machine-readable error code.
    pub fn code(&self) -> &'static str {
        match self {
            TokenError::Empty => "TOKEN_EMPTY",
            TokenError::NotThreeSegments => "TOKEN_NOT_THREE_SEGMENTS",
            TokenError::BadBase64(_) => "TOKEN_BAD_BASE64",
            TokenError::BadJson(_) => "TOKEN_BAD_JSON",
            TokenError::MissingClientId => "TOKEN_MISSING_CLIENT_ID",
            TokenError::MissingExp => "TOKEN_MISSING_EXP",
            TokenError::ExpNotNumber => "TOKEN_EXP_NOT_NUMBER",
        }
    }
}

/// Parsed claims from a Dhan access token payload.
///
/// Does **not** retain the raw token or signature.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenInfo {
    /// Dhan client ID, normalized to a String.
    pub client_id: String,

    /// Expiration time in Unix seconds (UTC).
    pub expires_at_unix: i64,

    /// All claims extracted from the token payload JSON.
    pub claims: serde_json::Map<String, serde_json::Value>,
}

impl TokenInfo {
    /// Parses a raw Dhan JWT string into [`TokenInfo`].
    ///
    /// Accepts surrounding whitespace, newlines, and stray padding characters.
    /// Normalizes numeric `dhanClientId` to a String.
    ///
    /// # Errors
    /// Returns [`TokenError`] if the token string cannot be decoded or is missing required claims.
    pub fn parse(token: &str) -> Result<Self, TokenError> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Err(TokenError::Empty);
        }

        let segments: Vec<&str> = trimmed.split('.').collect();
        if segments.len() != 3 {
            return Err(TokenError::NotThreeSegments);
        }

        let payload_raw = segments[1].trim();
        if payload_raw.is_empty() {
            return Err(TokenError::BadBase64("empty payload segment".into()));
        }

        // Tolerate padding and unpadded base64url inputs
        let unpadded = payload_raw.trim_end_matches('=');

        let payload_bytes = URL_SAFE_NO_PAD
            .decode(unpadded)
            .or_else(|_| {
                // If NO_PAD decode failed, try with restored canonical padding using URL_SAFE
                let mut padded = unpadded.to_string();
                while padded.len() % 4 != 0 {
                    padded.push('=');
                }
                URL_SAFE.decode(&padded)
            })
            .map_err(|e| TokenError::BadBase64(e.to_string()))?;

        let value: serde_json::Value = serde_json::from_slice(&payload_bytes)
            .map_err(|e| TokenError::BadJson(e.to_string()))?;

        let map = match value {
            serde_json::Value::Object(m) => m,
            _ => return Err(TokenError::BadJson("payload is not a JSON object".into())),
        };

        // Extract dhanClientId (string or number normalized to string)
        let client_id = match map.get("dhanClientId") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => return Err(TokenError::MissingClientId),
        };

        // Extract exp (Unix seconds)
        let exp_val = map.get("exp").ok_or(TokenError::MissingExp)?;
        let expires_at_unix = match exp_val {
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    i
                } else if let Some(f) = n.as_f64() {
                    if f.fract() == 0.0 && f >= (i64::MIN as f64) && f <= (i64::MAX as f64) {
                        f as i64
                    } else {
                        return Err(TokenError::ExpNotNumber);
                    }
                } else {
                    return Err(TokenError::ExpNotNumber);
                }
            }
            _ => return Err(TokenError::ExpNotNumber),
        };

        Ok(TokenInfo {
            client_id,
            expires_at_unix,
            claims: map,
        })
    }

    /// Exposes expiry as a UTC [`DateTime<Utc>`].
    ///
    /// Never panics, even on extreme timestamp bounds.
    pub fn expires_at(&self) -> DateTime<Utc> {
        DateTime::from_timestamp(self.expires_at_unix, 0).unwrap_or(DateTime::<Utc>::MAX_UTC)
    }

    /// Exposes expiry converted to Indian Standard Time (IST, Asia/Kolkata, UTC+05:30).
    pub fn expires_at_ist(&self) -> DateTime<FixedOffset> {
        self.expires_at().with_timezone(&ist_offset())
    }

    /// Formats the expiry timestamp in Indian Standard Time using a custom strftime format.
    pub fn format_ist(&self, format_str: &str) -> String {
        self.expires_at_ist().format(format_str).to_string()
    }

    /// Helper to format the expiry in IST as `YYYY-MM-DD HH:MM:SS IST`.
    pub fn expires_at_ist_string(&self) -> String {
        self.expires_at_ist().format("%Y-%m-%d %H:%M:%S IST").to_string()
    }

    /// Remaining time until expiration relative to `now`.
    ///
    /// Returns a negative duration if the token has already expired. Never panics.
    pub fn remaining(&self, now: DateTime<Utc>) -> Duration {
        self.expires_at() - now
    }

    /// Convenience wrapper returning remaining time relative to current system time.
    pub fn remaining_now(&self) -> Duration {
        self.remaining(Utc::now())
    }

    /// Checks whether the token is considered expired at time `now` given an explicit safety `margin`.
    ///
    /// Returns `true` if `remaining(now) <= margin`.
    ///
    /// ### Recommendation
    /// Callers are strongly advised to specify a margin of 1 to 2 minutes
    /// (`Duration::minutes(1)` or `Duration::minutes(2)`) to account for network latency,
    /// clock skew, and long-running operations.
    pub fn is_expired(&self, now: DateTime<Utc>, margin: Duration) -> bool {
        self.remaining(now) <= margin
    }

    /// Convenience wrapper checking expiration relative to current system time.
    pub fn is_expired_now(&self, margin: Duration) -> bool {
        self.is_expired(Utc::now(), margin)
    }

    /// Returns the token status relative to `now` and an explicit safety `margin`.
    ///
    /// - [`TokenStatus::Expired`] if `remaining <= margin`, with `since = margin - remaining`
    ///   (elapsed time since crossing the effective expiration threshold, >= 0).
    /// - [`TokenStatus::ExpiringSoon`] if `margin < remaining <= EXPIRING_SOON_THRESHOLD` (1 hour).
    /// - [`TokenStatus::Valid`] if `remaining > EXPIRING_SOON_THRESHOLD`.
    pub fn status(&self, now: DateTime<Utc>, margin: Duration) -> TokenStatus {
        let rem = self.remaining(now);
        if rem <= margin {
            let since = margin - rem;
            TokenStatus::Expired { since }
        } else if rem <= EXPIRING_SOON_THRESHOLD {
            TokenStatus::ExpiringSoon { remaining: rem }
        } else {
            TokenStatus::Valid { remaining: rem }
        }
    }

    /// Convenience wrapper evaluating status relative to current system time.
    pub fn status_now(&self, margin: Duration) -> TokenStatus {
        self.status(Utc::now(), margin)
    }

    /// Retrieve an arbitrary claim from the payload map by key.
    pub fn get_claim(&self, key: &str) -> Option<&serde_json::Value> {
        self.claims.get(key)
    }

    /// View all payload claims.
    pub fn claims(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.claims
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Constructs a fake JWT with a made-up signature. Never use real credentials.
    fn make_test_jwt(client_id_val: serde_json::Value, exp_val: Option<serde_json::Value>) -> String {
        let header = r#"{"alg":"HS256","typ":"JWT"}"#;
        let mut map = serde_json::Map::new();
        map.insert("dhanClientId".into(), client_id_val);
        if let Some(exp) = exp_val {
            map.insert("exp".into(), exp);
        }
        map.insert("customField".into(), serde_json::Value::String("extra_data".into()));

        let payload_json = serde_json::to_string(&map).unwrap();
        let h_b64 = URL_SAFE_NO_PAD.encode(header.as_bytes());
        let p_b64 = URL_SAFE_NO_PAD.encode(payload_json.as_bytes());
        format!("{h_b64}.{p_b64}.made_up_fake_signature_only")
    }

    #[test]
    fn test_valid_token_string_client_id() {
        let token = make_test_jwt(serde_json::json!("1000000001"), Some(serde_json::json!(1750000000)));
        let info = TokenInfo::parse(&token).expect("parse valid token");
        assert_eq!(info.client_id, "1000000001");
        assert_eq!(info.expires_at_unix, 1750000000);
        assert_eq!(info.expires_at().timestamp(), 1750000000);
        assert_eq!(info.get_claim("customField").unwrap(), "extra_data");
    }

    #[test]
    fn test_client_id_numeric_normalized_to_string() {
        let token = make_test_jwt(serde_json::json!(1000000002), Some(serde_json::json!(1750000000)));
        let info = TokenInfo::parse(&token).expect("parse valid numeric client ID");
        assert_eq!(info.client_id, "1000000002");
    }

    #[test]
    fn test_padding_whitespace_and_newlines() {
        let raw_token = make_test_jwt(serde_json::json!("12345"), Some(serde_json::json!(1750000000)));
        let parts: Vec<&str> = raw_token.split('.').collect();

        // Create padded variant
        let mut padded_payload = parts[1].to_string();
        while padded_payload.len() % 4 != 0 {
            padded_payload.push('=');
        }
        let token_with_padding = format!("{}.{}.{}", parts[0], padded_payload, parts[2]);

        // Wrap with whitespace and trailing newlines
        let messy_token = format!("\n  \t  {}  \r\n \n", token_with_padding);

        let info = TokenInfo::parse(&messy_token).expect("tolerate padding and surrounding whitespace");
        assert_eq!(info.client_id, "12345");
        assert_eq!(info.expires_at_unix, 1750000000);
    }

    #[test]
    fn test_error_empty() {
        assert_eq!(TokenInfo::parse("").unwrap_err(), TokenError::Empty);
        assert_eq!(TokenInfo::parse("   \r\n  ").unwrap_err(), TokenError::Empty);
        assert_eq!(TokenError::Empty.code(), "TOKEN_EMPTY");
    }

    #[test]
    fn test_error_not_three_segments() {
        assert_eq!(TokenInfo::parse("only_one_part").unwrap_err(), TokenError::NotThreeSegments);
        assert_eq!(TokenInfo::parse("part1.part2").unwrap_err(), TokenError::NotThreeSegments);
        assert_eq!(TokenInfo::parse("part1.part2.part3.part4").unwrap_err(), TokenError::NotThreeSegments);
        assert_eq!(TokenError::NotThreeSegments.code(), "TOKEN_NOT_THREE_SEGMENTS");
    }

    #[test]
    fn test_error_bad_base64() {
        let err = TokenInfo::parse("header.not!valid!base64$.signature").unwrap_err();
        assert!(matches!(err, TokenError::BadBase64(_)));
        assert_eq!(err.code(), "TOKEN_BAD_BASE64");

        // Empty payload segment is also a BadBase64 error
        let err_empty = TokenInfo::parse("header..signature").unwrap_err();
        assert!(matches!(err_empty, TokenError::BadBase64(_)));
    }

    #[test]
    fn test_error_bad_json() {
        let bad_json_b64 = URL_SAFE_NO_PAD.encode(b"not json at all");
        let token = format!("header.{}.signature", bad_json_b64);
        let err = TokenInfo::parse(&token).unwrap_err();
        assert!(matches!(err, TokenError::BadJson(_)));
        assert_eq!(err.code(), "TOKEN_BAD_JSON");

        // Valid JSON but not an object (e.g. array)
        let array_json_b64 = URL_SAFE_NO_PAD.encode(b"[1, 2, 3]");
        let token_array = format!("header.{}.signature", array_json_b64);
        let err_array = TokenInfo::parse(&token_array).unwrap_err();
        assert!(matches!(err_array, TokenError::BadJson(_)));
    }

    #[test]
    fn test_error_missing_client_id() {
        let header = r#"{"alg":"HS256"}"#;
        let payload = r#"{"exp":1750000000}"#; // missing dhanClientId
        let token = format!(
            "{}.{}.sig",
            URL_SAFE_NO_PAD.encode(header.as_bytes()),
            URL_SAFE_NO_PAD.encode(payload.as_bytes())
        );
        let err = TokenInfo::parse(&token).unwrap_err();
        assert_eq!(err, TokenError::MissingClientId);
        assert_eq!(err.code(), "TOKEN_MISSING_CLIENT_ID");
    }

    #[test]
    fn test_error_missing_exp() {
        let token = make_test_jwt(serde_json::json!("1000000001"), None);
        let err = TokenInfo::parse(&token).unwrap_err();
        assert_eq!(err, TokenError::MissingExp);
        assert_eq!(err.code(), "TOKEN_MISSING_EXP");
    }

    #[test]
    fn test_error_exp_not_number() {
        let token_str_exp = make_test_jwt(
            serde_json::json!("1000000001"),
            Some(serde_json::json!("1750000000")), // string instead of number
        );
        let err = TokenInfo::parse(&token_str_exp).unwrap_err();
        assert_eq!(err, TokenError::ExpNotNumber);
        assert_eq!(err.code(), "TOKEN_EXP_NOT_NUMBER");

        let token_bool_exp = make_test_jwt(
            serde_json::json!("1000000001"),
            Some(serde_json::json!(true)),
        );
        assert_eq!(TokenInfo::parse(&token_bool_exp).unwrap_err(), TokenError::ExpNotNumber);

        let token_fract_float = make_test_jwt(
            serde_json::json!("1000000001"),
            Some(serde_json::json!(1750000000.5)),
        );
        assert_eq!(TokenInfo::parse(&token_fract_float).unwrap_err(), TokenError::ExpNotNumber);
    }

    #[test]
    fn test_expiry_math_boundaries() {
        let exp_ts = 1_700_000_000i64;
        let token = make_test_jwt(serde_json::json!("1001"), Some(serde_json::json!(exp_ts)));
        let info = TokenInfo::parse(&token).unwrap();
        let exp_dt = DateTime::from_timestamp(exp_ts, 0).unwrap();

        // 1 second before exp
        let now_before = exp_dt - Duration::seconds(1);
        assert_eq!(info.remaining(now_before), Duration::seconds(1));
        assert!(!info.is_expired(now_before, Duration::zero()));
        assert_eq!(
            info.status(now_before, Duration::zero()),
            TokenStatus::ExpiringSoon {
                remaining: Duration::seconds(1)
            }
        );

        // Exactly at exp
        let now_exact = exp_dt;
        assert_eq!(info.remaining(now_exact), Duration::zero());
        assert!(info.is_expired(now_exact, Duration::zero()));
        assert_eq!(
            info.status(now_exact, Duration::zero()),
            TokenStatus::Expired {
                since: Duration::zero()
            }
        );

        // 1 second after exp
        let now_after = exp_dt + Duration::seconds(1);
        assert_eq!(info.remaining(now_after), Duration::seconds(-1));
        assert!(info.is_expired(now_after, Duration::zero()));
        assert_eq!(
            info.status(now_after, Duration::zero()),
            TokenStatus::Expired {
                since: Duration::seconds(1)
            }
        );

        // With margin: now is 30s before exp, margin is 60s (margin > remaining)
        let now_margin = exp_dt - Duration::seconds(30);
        assert_eq!(info.remaining(now_margin), Duration::seconds(30));
        assert!(info.is_expired(now_margin, Duration::seconds(60)));
        assert_eq!(
            info.status(now_margin, Duration::seconds(60)),
            TokenStatus::Expired {
                since: Duration::seconds(30)
            }
        );

        // With margin: now is 90s before exp, margin is 60s (remaining > margin)
        let now_safe = exp_dt - Duration::seconds(90);
        assert_eq!(info.remaining(now_safe), Duration::seconds(90));
        assert!(!info.is_expired(now_safe, Duration::seconds(60)));
    }

    #[test]
    fn test_expiring_soon_threshold_boundary() {
        let exp_ts = 1_700_000_000i64;
        let token = make_test_jwt(serde_json::json!("1001"), Some(serde_json::json!(exp_ts)));
        let info = TokenInfo::parse(&token).unwrap();
        let exp_dt = DateTime::from_timestamp(exp_ts, 0).unwrap();

        // 1 second above threshold (1 hour + 1 second remaining) -> Valid
        let now_above = exp_dt - EXPIRING_SOON_THRESHOLD - Duration::seconds(1);
        assert_eq!(
            info.status(now_above, Duration::zero()),
            TokenStatus::Valid {
                remaining: EXPIRING_SOON_THRESHOLD + Duration::seconds(1)
            }
        );

        // Exactly at threshold (1 hour remaining) -> ExpiringSoon
        let now_exact = exp_dt - EXPIRING_SOON_THRESHOLD;
        assert_eq!(
            info.status(now_exact, Duration::zero()),
            TokenStatus::ExpiringSoon {
                remaining: EXPIRING_SOON_THRESHOLD
            }
        );

        // 1 second below threshold (1 hour - 1 second remaining) -> ExpiringSoon
        let now_below = exp_dt - EXPIRING_SOON_THRESHOLD + Duration::seconds(1);
        assert_eq!(
            info.status(now_below, Duration::zero()),
            TokenStatus::ExpiringSoon {
                remaining: EXPIRING_SOON_THRESHOLD - Duration::seconds(1)
            }
        );
    }

    #[test]
    fn test_ist_display_formatting_and_date_rollover() {
        // UTC: 2026-11-20 20:30:00 UTC
        // IST: 2026-11-21 02:00:00 IST (+05:30)
        // Notice the dates differ: 2026-11-20 in UTC vs 2026-11-21 in IST.
        let dt_utc = DateTime::parse_from_rfc3339("2026-11-20T20:30:00Z").unwrap().with_timezone(&Utc);
        let token = make_test_jwt(serde_json::json!("CLIENT1"), Some(serde_json::json!(dt_utc.timestamp())));
        let info = TokenInfo::parse(&token).unwrap();

        assert_eq!(info.expires_at().format("%Y-%m-%d").to_string(), "2026-11-20");
        assert_eq!(info.format_ist("%Y-%m-%d"), "2026-11-21");
        assert_eq!(info.format_ist("%H:%M:%S"), "02:00:00");
        assert_eq!(info.expires_at_ist_string(), "2026-11-21 02:00:00 IST");
    }

    #[test]
    fn test_no_token_leakage_in_debug_and_display() {
        let marker = "SECRET_MARKER_DO_NOT_LEAK_XYZ_987";
        let fake_sig = format!("sig_{}", marker);

        // 1. Valid token: ensure TokenInfo does not store raw token or signature
        let header = r#"{"alg":"HS256"}"#;
        let payload = format!(r#"{{"dhanClientId":"CL123","exp":1800000000,"note":"{marker}"}}"#);
        let raw_token = format!(
            "{}.{}.{}",
            URL_SAFE_NO_PAD.encode(header.as_bytes()),
            URL_SAFE_NO_PAD.encode(payload.as_bytes()),
            fake_sig
        );
        let info = TokenInfo::parse(&raw_token).unwrap();
        let info_debug = format!("{:?}", info);
        assert!(!info_debug.contains(&fake_sig));
        assert!(!info_debug.contains(&raw_token));

        // 2. TokenError debug and display must never leak token or signature
        let test_cases = vec![
            format!("{marker}.incomplete"),
            format!("{marker}.bad!base64!val.{fake_sig}"),
            format!("hdr.{}.{fake_sig}", URL_SAFE_NO_PAD.encode(format!("bad json {marker}").as_bytes())),
            format!("hdr.{}.{fake_sig}", URL_SAFE_NO_PAD.encode(b"{\"exp\":\"not_num\"}")),
        ];

        for bad_token in test_cases {
            let err = TokenInfo::parse(&bad_token).unwrap_err();
            let debug_out = format!("{:?}", err);
            let display_out = format!("{}", err);

            assert!(
                !debug_out.contains(marker),
                "Debug output leaked marker: {debug_out}"
            );
            assert!(
                !display_out.contains(marker),
                "Display output leaked marker: {display_out}"
            );
            assert!(
                !debug_out.contains(&fake_sig),
                "Debug output leaked signature: {debug_out}"
            );
            assert!(
                !display_out.contains(&fake_sig),
                "Display output leaked signature: {display_out}"
            );
        }
    }
}
