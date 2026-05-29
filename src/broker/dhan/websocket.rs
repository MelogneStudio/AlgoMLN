use anyhow::Result;

use crate::models::Tick;

#[derive(Debug, Clone)]
pub struct DhanWebSocketConfig {
    pub client_id: String,
    pub access_token: String,
}

#[derive(Debug)]
pub struct DhanWebSocket {
    config: DhanWebSocketConfig,
}

impl DhanWebSocket {
    pub fn new(config: DhanWebSocketConfig) -> Self {
        Self { config }
    }

    pub async fn connect(&self) -> Result<()> {
        let _ = &self.config;
        Ok(())
    }

    pub async fn subscribe(&self, symbols: &[String]) -> Result<()> {
        let _ = symbols;
        Ok(())
    }

    pub async fn unsubscribe(&self, symbols: &[String]) -> Result<()> {
        let _ = symbols;
        Ok(())
    }

    pub async fn next_tick(&self) -> Result<Option<Tick>> {
        Ok(None)
    }
}
