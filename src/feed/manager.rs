use std::collections::HashSet;

use anyhow::{bail, Result};
use tokio::sync::broadcast;

use crate::models::Tick;

const MAX_DHAN_SUBSCRIPTIONS: usize = 1_000;
const TICK_CHANNEL_CAPACITY: usize = 4_096;

#[derive(Debug)]
pub struct FeedManager {
    subscriptions: HashSet<String>,
    sender: broadcast::Sender<Tick>,
}

impl Default for FeedManager {
    fn default() -> Self {
        Self::new()
    }
}

impl FeedManager {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(TICK_CHANNEL_CAPACITY);
        Self {
            subscriptions: HashSet::new(),
            sender,
        }
    }

    pub async fn subscribe(&mut self, symbols: Vec<String>) -> Result<()> {
        let new_symbols = symbols
            .into_iter()
            .map(|symbol| symbol.trim().to_string())
            .filter(|symbol| !symbol.is_empty())
            .collect::<Vec<_>>();

        let new_symbols = new_symbols.into_iter().collect::<HashSet<_>>();
        let new_total = self.subscriptions.union(&new_symbols).count();
        if new_total > MAX_DHAN_SUBSCRIPTIONS {
            bail!("Dhan feed supports up to {MAX_DHAN_SUBSCRIPTIONS} subscriptions");
        }

        self.subscriptions.extend(new_symbols);
        Ok(())
    }

    pub async fn unsubscribe(&mut self, symbols: Vec<String>) -> Result<()> {
        for symbol in symbols {
            self.subscriptions.remove(symbol.trim());
        }

        Ok(())
    }

    pub fn subscribe_ticks(&self) -> broadcast::Receiver<Tick> {
        self.sender.subscribe()
    }

    pub fn publish_tick(&self, tick: Tick) {
        let _ = self.sender.send(tick);
    }

    pub fn subscriptions(&self) -> &HashSet<String> {
        &self.subscriptions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fans_out_ticks_to_multiple_receivers() {
        let manager = FeedManager::new();
        let mut first = manager.subscribe_ticks();
        let mut second = manager.subscribe_ticks();

        manager.publish_tick(Tick {
            symbol: "1333".to_string(),
            ltp: 100.0,
            volume: 10,
            timestamp: 1,
        });

        assert_eq!(first.recv().await.unwrap().ltp, 100.0);
        assert_eq!(second.recv().await.unwrap().symbol, "1333");
    }
}
