use crate::exec::{
    correlation::CorrelationGenerator,
    normalize::NormalizedOrderUpdate,
    queue::{QueueAction, SubmissionQueue},
    reconciler::Reconciler,
    rest::DhanTradingClient,
    risk::{RiskController, RiskRailsConfig},
    state::{OrderState, OrderTracker},
    types::{
        ExecEvent, LocalOrderId, ModeConfig, OrderIntent, OrderKind, OrderRequest, Side,
    },
    wal::OrderWal,
};
use algomln::trader::{ExecutionPort, OrderEvent, OrderId, PortError};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
};

#[derive(Clone, Debug)]
pub struct DhanExecutionPortConfig {
    pub client_id: String,
    pub mode_config: ModeConfig,
    pub risk_config: RiskRailsConfig,
    pub wal_path: Option<PathBuf>,
    pub tick_sizes_paise: BTreeMap<String, i64>,
    pub run_tag: String,
}

pub struct DhanExecutionPort {
    client: DhanTradingClient,
    correlation_gen: CorrelationGenerator,
    queue: SubmissionQueue,
    wal: OrderWal,
    risk: RiskController,
    reconciler: Reconciler,
    local_orders: BTreeMap<LocalOrderId, OrderTracker>,
    local_positions: BTreeMap<String, i64>,
    tick_sizes_paise: BTreeMap<String, i64>,
    outgoing_events: VecDeque<ExecEvent>,
    seq_counter: u64,
    rt: Option<tokio::runtime::Runtime>,
}

impl DhanExecutionPort {
    pub fn new(config: DhanExecutionPortConfig) -> Result<Self, String> {
        let correlation_gen = CorrelationGenerator::new(&config.run_tag);
        let reconciler = Reconciler::new(correlation_gen.clone());
        let client = DhanTradingClient::new(config.mode_config.clone(), config.client_id.clone());
        let risk = RiskController::new(config.risk_config);

        let wal = if let Some(ref path) = config.wal_path {
            OrderWal::open(path)
                .map_err(|e| format!("failed opening WAL at {}: {e}", path.display()))?
        } else {
            OrderWal::memory()
        };

        // If tokio runtime is already present on thread, handle it, else create one for sync ExecutionPort bridge
        let rt = if tokio::runtime::Handle::try_current().is_ok() {
            None
        } else {
            Some(
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|e| format!("failed creating tokio runtime: {e}"))?,
            )
        };

        let mut port = Self {
            client,
            correlation_gen,
            queue: SubmissionQueue::default(),
            wal,
            risk,
            reconciler,
            local_orders: BTreeMap::new(),
            local_positions: BTreeMap::new(),
            tick_sizes_paise: config.tick_sizes_paise,
            outgoing_events: VecDeque::new(),
            seq_counter: 0,
            rt,
        };

        // Recover existing open orders from WAL on startup (Section 9)
        if let Ok(recovered) = port.wal.recover_open_orders() {
            for tracker in recovered {
                port.local_orders.insert(tracker.local_id.clone(), tracker);
            }
        }

        // Run startup check
        Self::run_future(&port.rt, async {
            port.client.startup_check().await
        }).map_err(|e| format!("startup check failed: {e}"))?;

        // Reconcile open orders with broker on startup
        let client = port.client.clone();
        let mut risk = std::mem::replace(&mut port.risk, RiskController::new(RiskRailsConfig::default()));
        let mut local_orders = std::mem::take(&mut port.local_orders);
        let mut local_positions = std::mem::take(&mut port.local_positions);

        let evs = Self::run_future(&port.rt, async {
            port.reconciler
                .reconcile(
                    &client,
                    &mut risk,
                    &mut local_orders,
                    &mut local_positions,
                )
                .await
        });
        port.risk = risk;
        port.local_orders = local_orders;
        port.local_positions = local_positions;
        port.outgoing_events.extend(evs);

        Ok(port)
    }

    fn run_future<F: std::future::Future>(rt_opt: &Option<tokio::runtime::Runtime>, f: F) -> F::Output {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            tokio::task::block_in_place(|| handle.block_on(f))
        } else if let Some(ref rt) = rt_opt {
            rt.block_on(f)
        } else {
            panic!("no tokio runtime available for DhanExecutionPort");
        }
    }

    /// Submit order according to Section 3:
    /// Returns LocalOrderId immediately, never blocks on fill.
    pub fn submit_order(&mut self, req: OrderRequest) -> Result<LocalOrderId, String> {
        self.risk.validate_order(&req)?;

        let local_id = req.local_id.clone();
        self.seq_counter += 1;
        let correlation_id = self.correlation_gen.generate(self.seq_counter);

        let mut tracker = OrderTracker::new(req.clone(), correlation_id.clone());
        tracker.mark_submitting();
        let _ = self.wal.append(&tracker);
        self.local_orders.insert(local_id.clone(), tracker);

        self.queue.enqueue_place(req)?;
        self.process_queue_step();

        Ok(local_id)
    }

    /// Cancel order according to Section 3:
    /// Cancel is a request; result arrives as an event.
    pub fn cancel_order_request(&mut self, local_id: &str) -> Result<(), String> {
        let tracker = self
            .local_orders
            .get(local_id)
            .ok_or_else(|| format!("unknown local order id: {local_id}"))?;

        if tracker.state.is_terminal() {
            return Ok(()); // Absorbs late cancel
        }

        let dhan_id = tracker
            .dhan_order_id
            .clone()
            .unwrap_or_else(|| "UNKNOWN".into());

        self.queue
            .enqueue_cancel(local_id.to_string(), dhan_id);
        self.process_queue_step();

        Ok(())
    }

    /// Process ready items from the prioritized rate-limited submission queue.
    pub fn process_queue_step(&mut self) {
        while !self.queue.is_empty() && self.queue.try_acquire_token() {
            let Some(action) = self.queue.pop_next() else {
                break;
            };

            match action {
                QueueAction::Place(req) => {
                    let tick = self
                        .tick_sizes_paise
                        .get(&req.security_id)
                        .copied()
                        .unwrap_or(5);

                    let correlation_id = self
                        .local_orders
                        .get(&req.local_id)
                        .map(|t| t.correlation_id.clone())
                        .unwrap_or_else(|| self.correlation_gen.generate(self.seq_counter));

                    let client = self.client.clone();
                    let res = Self::run_future(&self.rt, async {
                        client
                            .place_order(&req, &correlation_id, Some(tick))
                            .await
                    });

                    match res {
                        Ok(resp) => {
                            if let Some(tracker) = self.local_orders.get_mut(&req.local_id) {
                                let sub_ev = tracker.mark_submitted(Some(resp.order_id));
                                let _ = self.wal.append(tracker);
                                self.outgoing_events.push_back(sub_ev);
                            }
                        }
                        Err(crate::exec::rest::ExecRestError::Rejected { class, message, .. }) => {
                            if let Some(tracker) = self.local_orders.get_mut(&req.local_id) {
                                tracker.state = OrderState::Rejected;
                                let _ = self.wal.append(tracker);
                                self.outgoing_events.push_back(ExecEvent::Rejected {
                                    local_id: req.local_id.clone(),
                                    reason: message,
                                    error_class: class,
                                });
                            }
                        }
                        Err(crate::exec::rest::ExecRestError::Throttle(_)) => {
                            // Requeue throttled order
                            let _ = self.queue.enqueue_place(req);
                            break;
                        }
                        Err(other) => {
                            if let Some(tracker) = self.local_orders.get_mut(&req.local_id) {
                                tracker.state = OrderState::Unknown;
                                let _ = self.wal.append(tracker);
                                self.outgoing_events.push_back(ExecEvent::Unknown {
                                    local_id: req.local_id.clone(),
                                    reason: other.to_string(),
                                });
                            }
                        }
                    }
                }
                QueueAction::Cancel {
                    local_id,
                    dhan_order_id,
                } => {
                    let client = self.client.clone();
                    let res = Self::run_future(&self.rt, async {
                        client.cancel_order(&dhan_order_id).await
                    });

                    if res.is_ok() {
                        if let Some(tracker) = self.local_orders.get_mut(&local_id) {
                            tracker.state = OrderState::Cancelled;
                            let _ = self.wal.append(tracker);
                            self.outgoing_events.push_back(ExecEvent::Cancelled {
                                local_id: local_id.clone(),
                                remaining_qty: tracker.remaining_qty,
                            });
                        }
                    }
                }
            }
        }
    }

    /// Apply an incoming NormalizedOrderUpdate from poll or WebSocket feed.
    pub fn apply_update(&mut self, update: &NormalizedOrderUpdate) {
        // Find matching tracker by correlationId or dhan_order_id
        let tracker_opt = self
            .local_orders
            .values_mut()
            .find(|t| {
                if let Some(ref cid) = update.correlation_id {
                    t.correlation_id == *cid
                } else if let Some(ref did) = update.dhan_order_id {
                    t.dhan_order_id.as_deref() == Some(did.as_str())
                } else {
                    false
                }
            });

        if let Some(tracker) = tracker_opt {
            let evs = tracker.apply_update(update);
            let _ = self.wal.append(tracker);

            // Update local positions on fills
            for ev in &evs {
                if let ExecEvent::Fill { qty, .. } = ev {
                    let signed = match tracker.request.side {
                        Side::Buy => *qty as i64,
                        Side::Sell => -(*qty as i64),
                    };
                    *self
                        .local_positions
                        .entry(tracker.request.security_id.clone())
                        .or_default() += signed;
                }
            }

            self.outgoing_events.extend(evs);
        }
    }

    /// Snapshot returns open orders, positions, and trades (Section 3).
    pub fn snapshot(&self) -> (Vec<OrderTracker>, BTreeMap<String, i64>) {
        let open_orders = self
            .local_orders
            .values()
            .filter(|o| o.state.is_open_or_pending())
            .cloned()
            .collect();
        (open_orders, self.local_positions.clone())
    }

    /// Drain incoming raw ExecEvents
    pub fn drain_exec_events(&mut self) -> Vec<ExecEvent> {
        self.outgoing_events.drain(..).collect()
    }
}

/// Adapt DhanExecutionPort to the existing 3c ExecutionPort trait (Section 3).
impl ExecutionPort for DhanExecutionPort {
    fn submit(&mut self, r: &algomln::trader::OrderRequest) -> Result<(), PortError> {
        let side = match r.side {
            algomln::trader::Side::Buy => Side::Buy,
            algomln::trader::Side::Sell => Side::Sell,
        };

        let intent = match r.reason {
            algomln::trader::OrderReason::StopLoss
            | algomln::trader::OrderReason::TakeProfit
            | algomln::trader::OrderReason::DailyLossFlatten => OrderIntent::Exit,
            algomln::trader::OrderReason::Rule(_) => OrderIntent::Entry,
        };

        let kind = OrderKind::Market;

        let dhan_req = OrderRequest {
            local_id: r.client_order_id.clone(),
            security_id: r.symbol.clone(),
            side,
            quantity: r.qty,
            kind,
            intent,
            local_seq: self.seq_counter + 1,
        };

        self.submit_order(dhan_req)
            .map(|_| ())
            .map_err(PortError::Message)
    }

    fn cancel(&mut self, id: OrderId) -> Result<(), PortError> {
        self.cancel_order_request(&id).map_err(PortError::Message)
    }

    fn drain_events(&mut self) -> Vec<OrderEvent> {
        self.process_queue_step();
        let exec_evs = self.drain_exec_events();
        let mut order_events = Vec::new();

        for ev in exec_evs {
            match ev {
                ExecEvent::Submitted { local_id, .. } | ExecEvent::Open { local_id, .. } => {
                    order_events.push(OrderEvent::Acked { id: local_id });
                }
                ExecEvent::Fill {
                    local_id,
                    qty,
                    price_paise,
                    ..
                } => {
                    let price = algomln::trader::paise_to_rupees(price_paise);
                    order_events.push(OrderEvent::PartialFill {
                        id: local_id,
                        qty,
                        price,
                        fees: 0.0,
                    });
                }
                ExecEvent::Filled { local_id } => {
                    if let Some(tracker) = self.local_orders.get(&local_id) {
                        let price_paise = tracker.last_avg_price_paise.unwrap_or(0);
                        let price = algomln::trader::paise_to_rupees(price_paise);
                        order_events.push(OrderEvent::Filled {
                            id: local_id,
                            qty: tracker.order_qty,
                            price,
                            fees: 0.0,
                        });
                    }
                }
                ExecEvent::Rejected { local_id, reason, .. } => {
                    order_events.push(OrderEvent::Rejected {
                        id: local_id,
                        reason,
                    });
                }
                ExecEvent::Cancelled { local_id, .. } => {
                    order_events.push(OrderEvent::Cancelled { id: local_id });
                }
                ExecEvent::Expired { local_id, .. } => {
                    order_events.push(OrderEvent::Expired { id: local_id });
                }
                ExecEvent::Unknown { local_id, reason } => {
                    order_events.push(OrderEvent::Rejected {
                        id: local_id,
                        reason: format!("unknown state: {reason}"),
                    });
                }
                ExecEvent::ExternalChange { .. } | ExecEvent::Halt { .. } => {
                    // Internal safety / risk events; logged and tracked internally
                }
            }
        }

        order_events
    }
}
