use std::env;

use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhanAuth {
    pub access_token: String,
}

impl DhanAuth {
    pub fn new(access_token: impl Into<String>) -> Result<Self> {
        let access_token = access_token.into();
        if access_token.trim().is_empty() {
            return Err(anyhow!("Dhan access token is not configured"));
        }

        Ok(Self { access_token })
    }

    pub fn from_env() -> Result<Self> {
        let access_token = env::var("DHAN_ACCESS_TOKEN")
            .or_else(|_| env::var("DHAN_TOKEN"))
            .context("Set DHAN_ACCESS_TOKEN or DHAN_TOKEN")?;

        Self::new(access_token)
    }
}
