use crate::exec::{
    correlation::CorrelationGenerator,
    rest::DhanTradingClient,
    risk::RiskController,
    state::{OrderState, OrderTracker},
    types::{ExecEvent, LocalOrderId, OrderIntent, OrderKind, OrderRequest, Side},
};
use std::collections::BTreeMap;

pub struct Reconciler {
    correlation_gen: CorrelationGenerator,
}

impl Reconciler {
    pub fn new(correlation_gen: CorrelationGenerator) -> Self {
        Self { correlation_gen }
    }

    /// Run full reconciliation step (Section 9):
    /// 1. Fetch broker orders (own prefix only) and positions.
    /// 2. Match broker orders against local order trackers.
    /// 3. Resolve any Unknown orders via correlation ID.
    /// 4. Compare net positions against local positions.
    /// 5. Detect external flatten.
    pub async fn reconcile(
        &self,
        client: &DhanTradingClient,
        risk: &mut RiskController,
        local_orders: &mut BTreeMap<LocalOrderId, OrderTracker>,
        local_positions: &mut BTreeMap<String, i64>,
    ) -> Vec<ExecEvent> {
        let mut events = Vec::new();

        // 1. Fetch broker orders and positions
        let broker_orders = match client.get_orders().await {
            Ok(orders) => orders,
            Err(_) => return events,
        };

        let broker_positions = match client.get_positions().await {
            Ok(positions) => positions,
            Err(_) => return events,
        };

        // 2. Resolve Unknown orders via correlation ID lookup
        for tracker in local_orders.values_mut() {
            if tracker.state == OrderState::Unknown {
                if let Ok(Some(resolved)) = client.get_order_by_correlation(&tracker.correlation_id).await {
                    let evs = tracker.apply_update(&resolved);
                    events.extend(evs);
                }
            }
        }

        // 3. Match own prefix broker orders into local trackers
        for b_order in broker_orders {
            let Some(ref cid) = b_order.correlation_id else {
                continue;
            };
            if !self.correlation_gen.is_own(cid) {
                continue; // Skip non-AlgoMLN orders
            }

            // Find local tracker matching correlation_id
            if let Some(tracker) = local_orders.values_mut().find(|t| t.correlation_id == *cid) {
                let evs = tracker.apply_update(&b_order);
                events.extend(evs);
            }
        }

        // 4. Position comparison & ExternalChange detection
        let mut broker_pos_map: BTreeMap<String, i64> = BTreeMap::new();
        for pos in broker_positions {
            if pos.product_type == "INTRADAY" {
                *broker_pos_map.entry(pos.security_id).or_default() += pos.net_qty;
            }
        }

        // Check each security in our local positions
        for (sec_id, local_net) in local_positions.iter_mut() {
            let broker_net = broker_pos_map.get(sec_id).copied().unwrap_or(0);

            // External flatten: local position was > 0, but broker position is 0 without own exit
            if *local_net != 0 && broker_net == 0 {
                risk.lock_entries("external flatten detected: positions went flat on broker");
                events.push(ExecEvent::ExternalChange {
                    security_id: sec_id.clone(),
                    net_qty: 0,
                });
                events.push(ExecEvent::Halt {
                    reason: "external flatten detected; entries locked".into(),
                });

                // Cancel own pending orders on this symbol
                for tracker in local_orders.values_mut() {
                    if tracker.request.security_id == *sec_id && tracker.state.is_open_or_pending() {
                        tracker.state = OrderState::Cancelled;
                        events.push(ExecEvent::Cancelled {
                            local_id: tracker.local_id.clone(),
                            remaining_qty: tracker.remaining_qty,
                        });
                    }
                }
                *local_net = 0;
                continue;
            }

            // General position mismatch:
            if *local_net != broker_net {
                risk.lock_entries("position mismatch between local ledger and broker");
                events.push(ExecEvent::ExternalChange {
                    security_id: sec_id.clone(),
                    net_qty: broker_net,
                });
                events.push(ExecEvent::Halt {
                    reason: format!(
                        "position mismatch for {sec_id}: local={local_net}, broker={broker_net}"
                    ),
                });
                *local_net = broker_net;
            }
        }

        events
    }

    /// Execute MAX_DAILY_LOSS breach orchestration (Section 10):
    /// 1. Engage local entry lock.
    /// 2. Cancel own pending orders.
    /// 3. Send exit orders with top queue priority.
    /// 4. Reconcile until there are no open orders and no positions.
    /// 5. If broker_kill_switch_on_breach: activate kill switch and verify ACTIVE.
    pub async fn execute_max_daily_loss_breach(
        &self,
        client: &DhanTradingClient,
        risk: &mut RiskController,
        local_orders: &mut BTreeMap<LocalOrderId, OrderTracker>,
        local_positions: &mut BTreeMap<String, i64>,
    ) -> Vec<ExecEvent> {
        let mut events = Vec::new();

        // 1. Engage entry lock
        risk.lock_entries("MAX_DAILY_LOSS breached: entries locked, flattening positions");
        events.push(ExecEvent::Halt {
            reason: "MAX_DAILY_LOSS breached".into(),
        });

        // 2. Cancel own pending orders
        for tracker in local_orders.values_mut() {
            if tracker.state.is_open_or_pending() {
                if let Some(ref dhan_id) = tracker.dhan_order_id {
                    let _ = client.cancel_order(dhan_id).await;
                }
                tracker.state = OrderState::Cancelled;
                events.push(ExecEvent::Cancelled {
                    local_id: tracker.local_id.clone(),
                    remaining_qty: tracker.remaining_qty,
                });
            }
        }

        // 3. Send exit orders for all non-flat positions
        let mut exit_seq = 90_000u64;
        for (sec_id, &net_qty) in local_positions.iter() {
            if net_qty != 0 {
                let side = if net_qty > 0 { Side::Sell } else { Side::Buy };
                let qty = net_qty.unsigned_abs();
                exit_seq += 1;

                let exit_req = OrderRequest {
                    local_id: format!("BREACH-EXIT-{sec_id}"),
                    security_id: sec_id.clone(),
                    side,
                    quantity: qty,
                    kind: OrderKind::Market,
                    intent: OrderIntent::Exit,
                    local_seq: exit_seq,
                };

                let cid = self.correlation_gen.generate(exit_seq);
                if let Ok(resp) = client.place_order(&exit_req, &cid, None).await {
                    let mut tracker = OrderTracker::new(exit_req.clone(), cid);
                    let sub_ev = tracker.mark_submitted(Some(resp.order_id));
                    events.push(sub_ev);
                    local_orders.insert(exit_req.local_id.clone(), tracker);
                }
            }
        }

        // 4. Reconcile until flat (or max 5 iterations in test/live loop)
        for _ in 0..5 {
            let reconcile_evs = self.reconcile(client, risk, local_orders, local_positions).await;
            events.extend(reconcile_evs);

            let all_flat = local_positions.values().all(|&q| q == 0);
            let no_open = local_orders.values().all(|o| o.state.is_terminal());
            if all_flat && no_open {
                break;
            }
        }

        // 5. If broker_kill_switch_on_breach is on: POST /killswitch?killSwitchStatus=ACTIVATE
        if risk.config().broker_kill_switch_on_breach {
            let _ = client.activate_kill_switch().await;
            // Poll expecting ACTIVE
            for _ in 0..3 {
                if let Ok(status) = client.get_kill_switch_status().await {
                    if status == "ACTIVE" {
                        break;
                    }
                }
            }
        }

        events
    }
}
