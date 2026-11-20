use crate::error::ApiError;
use std::env;

pub trait TokenProvider: Send + Sync {
    fn access_token(&self) -> Result<String, ApiError>;
}

#[derive(Clone, Debug)]
pub struct EnvTokenProvider {
    pub variable: String,
}

impl Default for EnvTokenProvider {
    fn default() -> Self {
        Self {
            variable: "DHAN_ACCESS_TOKEN".into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct KeychainEnvTokenProvider {
    pub service: String,
    pub account: String,
    pub variable: String,
}

impl Default for KeychainEnvTokenProvider {
    fn default() -> Self {
        Self {
            service: "algomln".into(),
            account: "dhan-access-token".into(),
            variable: "DHAN_ACCESS_TOKEN".into(),
        }
    }
}

impl TokenProvider for KeychainEnvTokenProvider {
    fn access_token(&self) -> Result<String, ApiError> {
        if let Ok(entry) = keyring::Entry::new(&self.service, &self.account) {
            if let Ok(token) = entry.get_password() {
                let trimmed = token.trim().to_string();
                if !trimmed.is_empty() {
                    return Ok(trimmed);
                }
            }
        }
        env::var(&self.variable)
            .map(|s| s.trim().to_string())
            .map_err(|_| {
                ApiError::Auth("Dhan token missing from OS keychain and DHAN_ACCESS_TOKEN".into())
            })
    }
}

impl TokenProvider for EnvTokenProvider {
    fn access_token(&self) -> Result<String, ApiError> {
        env::var(&self.variable)
            .map(|s| s.trim().to_string())
            .map_err(|_| {
                ApiError::Auth(
                    "missing access token; set DHAN_ACCESS_TOKEN or configure an OS keychain provider"
                        .into(),
                )
            })
    }
}
