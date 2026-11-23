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
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct ExitReplaceConfig {
    pub timeout: Duration,
    pub max_rounds: usize,
}

impl Default for ExitReplaceConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            max_rounds: 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExitLoopPhase {
    AwaitingPending,
    TimerRunning,
    AwaitingCancelConfirm,
    Reconciling,
    Terminal,
}

#[derive(Clone, Debug)]
pub struct ActiveExitLoop {
    pub symbol: String,
    pub original_local_id: LocalOrderId,
    pub current_local_id: LocalOrderId,
    pub round: usize,
    pub timer_started_at: Option<Instant>,
    pub phase: ExitLoopPhase,
}

impl ActiveExitLoop {
    pub fn is_terminal(&self) -> bool {
        matches!(self.phase, ExitLoopPhase::Terminal)
    }
}

#[derive(Clone, Debug)]
pub struct DhanExecutionPortConfig {
    pub client_id: String,
    pub mode_config: ModeConfig,
    pub risk_config: RiskRailsConfig,
    pub wal_path: Option<PathBuf>,
    pub tick_sizes_paise: BTreeMap<String, i64>,
    pub run_tag: String,
    pub exit_replace_config: ExitReplaceConfig,
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
    emitted_fill_qtys: BTreeMap<LocalOrderId, u64>,
    exit_replace_config: ExitReplaceConfig,
    active_exit_loops: BTreeMap<String, ActiveExitLoop>,
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

        // If tokio runtime is already present on thread, handle it, else create a multi-thread runtime
        let rt = if tokio::runtime::Handle::try_current().is_ok() {
            None
        } else {
            Some(
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
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
            emitted_fill_qtys: BTreeMap::new(),
            exit_replace_config: config.exit_replace_config,
            active_exit_loops: BTreeMap::new(),
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

    fn run_future<F: std::future::Future>(rt_opt: &Option<tokio::runtime::Runtime>, f: F) -> F::Output
    where
        F: Send,
        F::Output: Send,
    {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            std::thread::scope(|s| {
                s.spawn(|| handle.block_on(f)).join().unwrap()
            })
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

        // Rule 6: Invariant: at most one live exit order per symbol at any time.
        // Enforce it in the state machine and reject a second submit with a clear error.
        if req.intent == OrderIntent::Exit {
            let has_live_exit = self.local_orders.values().any(|tracker| {
                tracker.request.security_id == req.security_id
                    && tracker.request.intent == OrderIntent::Exit
                    && tracker.state.is_open_or_pending()
            }) || self
                .active_exit_loops
                .get(&req.security_id)
                .is_some_and(|l| !l.is_terminal());

            if has_live_exit {
                return Err(format!(
                    "rejected: active exit order already exists for symbol '{}'",
                    req.security_id
                ));
            }
        }

        let local_id = req.local_id.clone();
        self.seq_counter += 1;
        let correlation_id = self.correlation_gen.generate(self.seq_counter);

        let mut tracker = OrderTracker::new(req.clone(), correlation_id.clone());
        tracker.mark_submitting();
        let _ = self.wal.append(&tracker);
        self.local_orders.insert(local_id.clone(), tracker);

        // Track bounded exit replace loop only for EXIT orders (Rule 1 & Rule 6)
        if req.intent == OrderIntent::Exit {
            self.active_exit_loops.insert(
                req.security_id.clone(),
                ActiveExitLoop {
                    symbol: req.security_id.clone(),
                    original_local_id: local_id.clone(),
                    current_local_id: local_id.clone(),
                    round: 1,
                    timer_started_at: None,
                    phase: ExitLoopPhase::AwaitingPending,
                },
            );
        }

        self.queue.enqueue_place(req)?;
        // Non-blocking: returns LocalOrderId immediately per Section 3
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
                                if class.locks_entries() {
                                    self.risk.lock_entries(&format!("fatal rejection ({class:?}): {message}"));
                                    self.outgoing_events.push_back(ExecEvent::Halt {
                                        reason: format!("fatal config/auth rejection ({class:?}): {message}"),
                                    });
                                }
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
            let tr_local_id = tracker.local_id.clone();
            let tr_sec_id = tracker.request.security_id.clone();
            let evs = tracker.apply_update(update);
            let new_state = tracker.state;
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

            // Exit replace loop update (Rule 1 & Rule 3)
            if let Some(exit_loop) = self.active_exit_loops.get_mut(&tr_sec_id) {
                if exit_loop.current_local_id == tr_local_id && !exit_loop.is_terminal() {
                    match new_state {
                        OrderState::Open | OrderState::PartiallyFilled => {
                            if exit_loop.phase == ExitLoopPhase::AwaitingPending {
                                exit_loop.phase = ExitLoopPhase::TimerRunning;
                                exit_loop.timer_started_at = Some(Instant::now());
                            }
                        }
                        OrderState::Filled => {
                            println!(
                                "EXIT_REPLACE: symbol={}, round={}, old_order_id={}, outcome=TRADED, new_order_id=none",
                                exit_loop.symbol, exit_loop.round, exit_loop.current_local_id
                            );
                            exit_loop.phase = ExitLoopPhase::Terminal;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    pub fn active_exit_loops(&self) -> &BTreeMap<String, ActiveExitLoop> {
        &self.active_exit_loops
    }

    pub fn local_orders(&self) -> &BTreeMap<LocalOrderId, OrderTracker> {
        &self.local_orders
    }

    pub fn risk(&self) -> &RiskController {
        &self.risk
    }

    pub fn trigger_exit_timeout_for_test(&mut self, symbol: &str) {
        if let Some(loop_state) = self.active_exit_loops.get_mut(symbol) {
            if loop_state.phase == ExitLoopPhase::TimerRunning {
                loop_state.timer_started_at = Some(
                    Instant::now()
                        .checked_sub(Duration::from_secs(100))
                        .unwrap_or_else(Instant::now),
                );
            }
        }
    }

    /// Process exit replace loops according to Task 2 rules:
    /// 1. Timeout starts only when exit is PENDING (not TRANSIT).
    /// 2. On timeout: send cancel. Do NOT send replacement until old order is confirmed terminal.
    /// 3. If old order ended TRADED, stop.
    /// 4. Size replacement from broker-confirmed position snapshot, never exceeding open qty.
    /// 5. New correlation ID for every replacement.
    /// 7. If cancel fails or outcome unknown, run reconcile first. Never guess.
    /// 8. After max rounds, stop replacing, emit Halt with ExitNotFilled, alert.
    /// 9. Log each round: symbol, round, old order id, outcome, new order id.
    pub fn check_exit_timeouts(&mut self) {
        let symbols: Vec<String> = self.active_exit_loops.keys().cloned().collect();

        for symbol in symbols {
            let Some(mut exit_loop) = self.active_exit_loops.remove(&symbol) else {
                continue;
            };

            if exit_loop.is_terminal() {
                self.active_exit_loops.insert(symbol, exit_loop);
                continue;
            }

            let old_order_id = exit_loop.current_local_id.clone();

            // Phase 1: TimerRunning -> Check if timeout expired
            if exit_loop.phase == ExitLoopPhase::TimerRunning {
                let timed_out = exit_loop
                    .timer_started_at
                    .is_some_and(|t| t.elapsed() >= self.exit_replace_config.timeout);

                if timed_out {
                    let dhan_id_opt = self
                        .local_orders
                        .get(&old_order_id)
                        .and_then(|t| t.dhan_order_id.clone())
                        .filter(|id| !id.is_empty() && id != "UNKNOWN");

                    let client = self.client.clone();
                    let cancel_res = if let Some(ref did) = dhan_id_opt {
                        Self::run_future(&self.rt, async { client.cancel_order(did).await })
                    } else if let Some(t) = self.local_orders.get(&old_order_id) {
                        let cid = t.correlation_id.clone();
                        Self::run_future(&self.rt, async {
                            if let Ok(Some(norm)) = client.get_order_by_correlation(&cid).await {
                                if let Some(ref did) = norm.dhan_order_id {
                                    return client.cancel_order(did).await;
                                }
                            }
                            Err(crate::exec::rest::ExecRestError::Transport(
                                "order id not found for cancel".into(),
                            ))
                        })
                    } else {
                        Err(crate::exec::rest::ExecRestError::Transport(
                            "tracker not found".into(),
                        ))
                    };

                    match cancel_res {
                        Ok(()) => {
                            // Rule 2: Do NOT send the replacement until the old order is confirmed terminal
                            exit_loop.phase = ExitLoopPhase::AwaitingCancelConfirm;
                        }
                        Err(_) => {
                            // Rule 7: If cancel fails or outcome unknown, run reconcile first
                            exit_loop.phase = ExitLoopPhase::Reconciling;
                        }
                    }
                }
            }

            // Phase 2: AwaitingCancelConfirm or Reconciling -> Check/reconcile confirmed terminal status
            if exit_loop.phase == ExitLoopPhase::AwaitingCancelConfirm
                || exit_loop.phase == ExitLoopPhase::Reconciling
            {
                if exit_loop.phase == ExitLoopPhase::Reconciling {
                    // Run reconcile first (Rule 7)
                    let client = self.client.clone();
                    let mut risk = std::mem::replace(
                        &mut self.risk,
                        RiskController::new(RiskRailsConfig::default()),
                    );
                    let mut local_orders = std::mem::take(&mut self.local_orders);
                    let mut local_positions = std::mem::take(&mut self.local_positions);

                    let evs = Self::run_future(&self.rt, async {
                        self.reconciler
                            .reconcile(
                                &client,
                                &mut risk,
                                &mut local_orders,
                                &mut local_positions,
                            )
                            .await
                    });
                    self.risk = risk;
                    self.local_orders = local_orders;
                    self.local_positions = local_positions;
                    self.outgoing_events.extend(evs);
                } else {
                    // Query broker order status via correlation lookup
                    if let Some(tracker) = self.local_orders.get(&old_order_id) {
                        let client = self.client.clone();
                        let cid = tracker.correlation_id.clone();
                        let lookup = Self::run_future(&self.rt, async {
                            client.get_order_by_correlation(&cid).await
                        });
                        if let Ok(Some(update)) = lookup {
                            self.apply_update(&update);
                        }
                    }
                }

                // Check tracker state now
                let tracker_opt = self.local_orders.get(&old_order_id).cloned();
                if let Some(tracker) = tracker_opt {
                    if !tracker.state.is_terminal() {
                        // Old order is still open! Wait, do NOT send replacement on a guess (Rule 2 & 7)
                        self.active_exit_loops.insert(symbol, exit_loop);
                        continue;
                    }

                    let outcome = tracker.state;

                    // Rule 3: If old order ended TRADED, stop.
                    if outcome == OrderState::Filled {
                        println!(
                            "EXIT_REPLACE: symbol={}, round={}, old_order_id={}, outcome=TRADED, new_order_id=none",
                            symbol, exit_loop.round, old_order_id
                        );
                        exit_loop.phase = ExitLoopPhase::Terminal;
                        self.active_exit_loops.insert(symbol, exit_loop);
                        continue;
                    }

                    // Rule 8: After max rounds, stop replacing, emit Halt with ExitNotFilled, alert.
                    if exit_loop.round >= self.exit_replace_config.max_rounds {
                        println!(
                            "EXIT_REPLACE: symbol={}, round={}, old_order_id={}, outcome={:?}, new_order_id=none",
                            symbol, exit_loop.round, old_order_id, outcome
                        );
                        exit_loop.phase = ExitLoopPhase::Terminal;
                        self.outgoing_events.push_back(ExecEvent::Halt {
                            reason: "ExitNotFilled".to_string(),
                        });
                        self.risk.lock_entries("ExitNotFilled: max exit replace rounds reached");

                        if self.risk.config().allow_emergency_delete_all_positions {
                            let client = self.client.clone();
                            let _ = Self::run_future(&self.rt, async {
                                client.delete_all_positions().await
                            });
                        }

                        self.active_exit_loops.insert(symbol, exit_loop);
                        continue;
                    }

                    // Rule 4: Size replacement from broker-confirmed position snapshot
                    let client = self.client.clone();
                    let broker_positions = Self::run_future(&self.rt, async {
                        client.get_positions().await
                    })
                    .unwrap_or_default();

                    let mut confirmed_net_qty = 0i64;
                    for pos in broker_positions {
                        if pos.security_id == symbol && pos.product_type.eq_ignore_ascii_case("INTRADAY") {
                            confirmed_net_qty += pos.net_qty;
                        }
                    }

                    if confirmed_net_qty == 0 {
                        println!(
                            "EXIT_REPLACE: symbol={}, round={}, old_order_id={}, outcome={:?}, new_order_id=none",
                            symbol, exit_loop.round, old_order_id, outcome
                        );
                        exit_loop.phase = ExitLoopPhase::Terminal;
                        self.active_exit_loops.insert(symbol, exit_loop);
                        continue;
                    }

                    let replace_qty = confirmed_net_qty.unsigned_abs();
                    let replace_side = if confirmed_net_qty > 0 {
                        Side::Sell
                    } else {
                        Side::Buy
                    };

                    // Rule 5: New correlation ID for every replacement.
                    self.seq_counter += 1;
                    let new_cid = self.correlation_gen.generate(self.seq_counter);
                    let new_local_id = format!("{}-R{}", exit_loop.original_local_id, exit_loop.round + 1);

                    // Rule 9: Each round must log: symbol, round number, old order id, outcome, new order id.
                    println!(
                        "EXIT_REPLACE: symbol={}, round={}, old_order_id={}, outcome={:?}, new_order_id={}",
                        symbol, exit_loop.round, old_order_id, outcome, new_local_id
                    );

                    let replace_req = OrderRequest {
                        local_id: new_local_id.clone(),
                        security_id: symbol.clone(),
                        side: replace_side,
                        quantity: replace_qty,
                        kind: OrderKind::Market,
                        intent: OrderIntent::Exit,
                        local_seq: self.seq_counter,
                    };

                    let mut new_tracker = OrderTracker::new(replace_req.clone(), new_cid.clone());
                    new_tracker.mark_submitting();
                    let _ = self.wal.append(&new_tracker);
                    self.local_orders.insert(new_local_id.clone(), new_tracker);

                    exit_loop.round += 1;
                    exit_loop.current_local_id = new_local_id.clone();
                    exit_loop.timer_started_at = None;
                    exit_loop.phase = ExitLoopPhase::AwaitingPending;

                    let _ = self.queue.enqueue_place(replace_req);
                    self.process_queue_step();
                }
            }

            self.active_exit_loops.insert(symbol, exit_loop);
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
        self.check_exit_timeouts();
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
                    *self.emitted_fill_qtys.entry(local_id.clone()).or_default() += qty;
                    order_events.push(OrderEvent::PartialFill {
                        id: local_id,
                        qty,
                        price,
                        fees: 0.0,
                    });
                }
                ExecEvent::Filled { local_id } => {
                    if let Some(tracker) = self.local_orders.get(&local_id) {
                        let already_emitted = self.emitted_fill_qtys.get(&local_id).copied().unwrap_or(0);
                        let remaining_unemitted = tracker.order_qty.saturating_sub(already_emitted);
                        let price_paise = tracker.last_avg_price_paise.unwrap_or(0);
                        let price = algomln::trader::paise_to_rupees(price_paise);
                        if remaining_unemitted > 0 {
                            *self.emitted_fill_qtys.entry(local_id.clone()).or_default() += remaining_unemitted;
                            order_events.push(OrderEvent::Filled {
                                id: local_id,
                                qty: remaining_unemitted,
                                price,
                                fees: 0.0,
                            });
                        }
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
