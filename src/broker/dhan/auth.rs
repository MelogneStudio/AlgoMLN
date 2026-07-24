use std::env;

use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhanAuth {
    pub access_token: String,
    pub client_id: Option<String>,
}

impl DhanAuth {
    pub fn new(access_token: impl Into<String>) -> Result<Self> {
        let access_token = access_token.into();
        if access_token.trim().is_empty() {
            return Err(anyhow!("Dhan access token is not configured"));
        }

        Ok(Self {
            access_token,
            client_id: None,
        })
    }

    pub fn with_client_id(
        access_token: impl Into<String>,
        client_id: impl Into<String>,
    ) -> Result<Self> {
        let mut auth = Self::new(access_token)?;
        let client_id = client_id.into();
        if client_id.trim().is_empty() {
            return Err(anyhow!("Dhan client id is not configured"));
        }
        auth.client_id = Some(client_id);
        Ok(auth)
    }

    pub fn from_env() -> Result<Self> {
        let access_token = env::var("DHAN_ACCESS_TOKEN")
            .or_else(|_| env::var("DHAN_TOKEN"))
            .context("Set DHAN_ACCESS_TOKEN or DHAN_TOKEN")?;

        let mut auth = Self::new(access_token)?;
        auth.client_id = env::var("DHAN_CLIENT_ID")
            .or_else(|_| env::var("DHAN_CLIENTID"))
            .ok();
        Ok(auth)
    }
}
