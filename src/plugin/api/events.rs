use std::sync::Arc;

use parking_lot::RwLock;

use crate::models::Candle;
use crate::plugin::types::SubscriptionHandle;
use crate::strategy::execution::paper::PaperTrade;

// NOTE: `crate::models::Candle` already derives `Clone`. `PaperTrade` derives
// `Clone` (see src/strategy/execution/paper.rs), so `derive(Clone)` on
// `EventKind` is sound.

#[derive(Debug, Clone)]
pub enum EventKind {
    CandleProcessed(Candle),
    TradeExecuted(PaperTrade),
    RuleFired { rule_id: String, strategy_id: String },
    StrategyStatusChanged { strategy_id: String, new_status: String },
    SystemShutdown,
}

#[derive(Debug, Clone, Copy)]
pub enum EventFilter {
    All,
    CandleProcessed,
    TradeExecuted,
    RuleFired,
    StrategyStatusChanged,
    SystemShutdown,
}

impl EventFilter {
    fn matches(&self, event: &EventKind) -> bool {
        match (self, event) {
            (EventFilter::All, _) => true,
            (EventFilter::CandleProcessed, EventKind::CandleProcessed(_)) => true,
            (EventFilter::TradeExecuted, EventKind::TradeExecuted(_)) => true,
            (EventFilter::RuleFired, EventKind::RuleFired { .. }) => true,
            (EventFilter::StrategyStatusChanged, EventKind::StrategyStatusChanged { .. }) => true,
            (EventFilter::SystemShutdown, EventKind::SystemShutdown) => true,
            _ => false,
        }
    }
}

pub struct EventBus {
    subscribers: Arc<
        RwLock<
            Vec<(
                SubscriptionHandle,
                EventFilter,
                Arc<dyn Fn(EventKind) + Send + Sync>,
            )>,
        >,
    >,
}

impl EventBus {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            subscribers: Arc::new(RwLock::new(Vec::new())),
        })
    }

    pub fn subscribe(
        &self,
        filter: EventFilter,
        callback: Arc<dyn Fn(EventKind) + Send + Sync>,
    ) -> SubscriptionHandle {
        let handle = SubscriptionHandle(uuid::Uuid::new_v4());
        let mut guard = self.subscribers.write();
        guard.push((handle, filter, callback));
        handle
    }

    pub fn unsubscribe(&self, handle: &SubscriptionHandle) {
        self.subscribers.write().retain(|(h, _, _)| h != handle);
    }

    pub fn publish(&self, event: EventKind) {
        // Collect matching (callback, event_clone) tuples under the read lock,
        // then release the lock before spawning tasks.
        let to_spawn: Vec<Arc<dyn Fn(EventKind) + Send + Sync>> = {
            let guard = self.subscribers.read();
            guard
                .iter()
                .filter(|(_, filter, _)| filter.matches(&event))
                .map(|(_, _, cb)| cb.clone())
                .collect()
        };
        for cb in to_spawn {
            let evt = event.clone();
            tokio::spawn(async move {
                cb(evt);
            });
        }
    }
}
