use crate::trader::types::{OrderEvent, OrderId, OrderRequest, PortError};
use std::collections::VecDeque;

pub trait ExecutionPort {
    fn submit(&mut self, req: &OrderRequest) -> Result<(), PortError>;
    fn cancel(&mut self, id: OrderId) -> Result<(), PortError>;
    fn drain_events(&mut self) -> Vec<OrderEvent>;
}

/// Deterministic execution test double. Tests enqueue delayed events explicitly.
#[derive(Default)]
pub struct ScriptedPort {
    pub submitted: Vec<OrderRequest>,
    pub cancelled: Vec<OrderId>,
    events: VecDeque<OrderEvent>,
}

impl ScriptedPort {
    pub fn push(&mut self, event: OrderEvent) {
        self.events.push_back(event)
    }

    pub fn push_all(&mut self, events: impl IntoIterator<Item = OrderEvent>) {
        self.events.extend(events)
    }
}

impl ExecutionPort for ScriptedPort {
    fn submit(&mut self, r: &OrderRequest) -> Result<(), PortError> {
        self.submitted.push(r.clone());
        Ok(())
    }

    fn cancel(&mut self, id: OrderId) -> Result<(), PortError> {
        self.cancelled.push(id);
        Ok(())
    }

    fn drain_events(&mut self) -> Vec<OrderEvent> {
        self.events.drain(..).collect()
    }
}
