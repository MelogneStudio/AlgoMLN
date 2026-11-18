use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TokenClaims {
    pub client_id: String,
    pub exp: i64,
}

impl TokenClaims {
    pub fn is_expired(&self, now: i64) -> bool {
        self.exp <= now
    }

    pub fn time_to_expiry(&self, now: i64) -> i64 {
        self.exp - now
    }
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum AuthError {
    #[error("token is not a valid JWT (expected 3 parts separated by '.')")]
    InvalidFormat,
    #[error("failed to decode JWT payload base64: {0}")]
    Base64(String),
    #[error("failed to parse JWT payload JSON: {0}")]
    Json(String),
    #[error("JWT payload missing 'exp' claim")]
    MissingExp,
    #[error("JWT payload missing 'dhanClientId' claim")]
    MissingClientId,
    #[error("Dhan access token expired at {exp} (current time: {now})")]
    Expired { exp: i64, now: i64 },
}

/// Parses the payload of a Dhan JWT token without verifying its signature.
/// Validates that `dhanClientId` and `exp` exist, and that `exp > now`.
pub fn parse_and_validate_token(token: &str, now: i64) -> Result<TokenClaims, AuthError> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err(AuthError::InvalidFormat);
    }

    let payload_str = parts[1];
    let decoded_bytes = URL_SAFE_NO_PAD
        .decode(payload_str.as_bytes())
        .or_else(|_| {
            // Try with standard padding if needed
            let mut padded = payload_str.to_string();
            while padded.len() % 4 != 0 {
                padded.push('=');
            }
            URL_SAFE_NO_PAD.decode(padded.as_bytes())
        })
        .map_err(|e| AuthError::Base64(e.to_string()))?;

    let json_val: Value = serde_json::from_slice(&decoded_bytes)
        .map_err(|e| AuthError::Json(e.to_string()))?;

    let exp = json_val
        .get("exp")
        .and_then(|v| v.as_i64())
        .ok_or(AuthError::MissingExp)?;

    let client_id = match json_val.get("dhanClientId") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return Err(AuthError::MissingClientId),
    };

    let claims = TokenClaims { client_id, exp };

    if claims.is_expired(now) {
        return Err(AuthError::Expired { exp, now });
    }

    Ok(claims)
}

/// Constructs the WebSocket URL from token and client ID.
/// NOTE: The URL contains the token and must never be logged.
pub fn build_feed_url(base_ws_url: &str, token: &str, client_id: &str) -> String {
    let base = base_ws_url.trim_end_matches('/');
    format!(
        "{base}/?version=2&token={token}&clientId={client_id}&authType=2"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_jwt(client_id: &str, exp: i64) -> String {
        let header = r#"{"alg":"HS256","typ":"JWT"}"#;
        let payload = format!(r#"{{"dhanClientId":"{}","exp":{}}}"#, client_id, exp);
        let h_b64 = URL_SAFE_NO_PAD.encode(header.as_bytes());
        let p_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
        format!("{h_b64}.{p_b64}.fakesignature")
    }

    #[test]
    fn test_valid_token() {
        let token = create_test_jwt("1000000001", 1750000000);
        let claims = parse_and_validate_token(&token, 1700000000).expect("valid token");
        assert_eq!(claims.client_id, "1000000001");
        assert_eq!(claims.exp, 1750000000);
        assert_eq!(claims.time_to_expiry(1700000000), 50000000);
    }

    #[test]
    fn test_expired_token() {
        let token = create_test_jwt("1000000001", 1690000000);
        let err = parse_and_validate_token(&token, 1700000000).unwrap_err();
        assert_eq!(
            err,
            AuthError::Expired {
                exp: 1690000000,
                now: 1700000000
            }
        );
    }

    #[test]
    fn test_numeric_client_id() {
        let header = r#"{"alg":"HS256","typ":"JWT"}"#;
        let payload = r#"{"dhanClientId":1000000002,"exp":1750000000}"#;
        let token = format!(
            "{}.{}.sig",
            URL_SAFE_NO_PAD.encode(header.as_bytes()),
            URL_SAFE_NO_PAD.encode(payload.as_bytes())
        );
        let claims = parse_and_validate_token(&token, 1700000000).expect("valid numeric id");
        assert_eq!(claims.client_id, "1000000002");
    }

    #[test]
    fn test_malformed_tokens() {
        assert_eq!(
            parse_and_validate_token("not-a-jwt", 1000).unwrap_err(),
            AuthError::InvalidFormat
        );

        let bad_base64 = "aaa.!!!.bbb";
        assert!(matches!(
            parse_and_validate_token(bad_base64, 1000).unwrap_err(),
            AuthError::Base64(_)
        ));

        let bad_json = format!(
            "aaa.{}.bbb",
            URL_SAFE_NO_PAD.encode("not valid json".as_bytes())
        );
        assert!(matches!(
            parse_and_validate_token(&bad_json, 1000).unwrap_err(),
            AuthError::Json(_)
        ));
    }
}
