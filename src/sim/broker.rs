use std::collections::{BTreeMap, BTreeSet};

use crate::{
    engine::Symbol,
    indicators::Candle,
    trader::{ExecutionPort, OrderEvent, OrderId, OrderRequest, PortError, Side},
};
use super::types::{round_half_even_f64, round_half_even_i128, FeeModel};

#[derive(Clone, Debug)]
pub struct SimBroker {
    pub symbols: Vec<Symbol>,
    pub latency: u64,
    pub slippage_bps: f64,
    /// Minimum permitted fill-price increment in paise (NSE equity default: 5).
    pub tick_size_paise: i64,
    pub tick_sizes_paise: BTreeMap<Symbol, i64>,
    pub partial_fills_pct: Option<f64>,
    pub fees: FeeModel,
    pending: BTreeMap<OrderId, PendingOrder>,
    events: Vec<OrderEvent>,
    ticks: BTreeMap<Symbol, u64>,
    seen_ids: BTreeSet<OrderId>,
}

#[derive(Clone, Debug)]
struct PendingOrder {
    req: OrderRequest,
    remaining: u64,
    due: u64,
    acked: bool,
    notional_filled_paise: i128,
    fees_charged_paise: i64,
}

impl SimBroker {
    pub fn new(symbols: Vec<Symbol>) -> Self {
        Self {
            symbols,
            latency: 1,
            slippage_bps: 0.0,
            tick_size_paise: 5,
            tick_sizes_paise: BTreeMap::new(),
            partial_fills_pct: None,
            fees: FeeModel::default(),
            pending: BTreeMap::new(),
            events: Vec::new(),
            ticks: BTreeMap::new(),
            seen_ids: BTreeSet::new(),
        }
    }

    pub fn on_candle_open(&mut self, candles: &BTreeMap<Symbol, Candle>) {
        for s in candles.keys() {
            *self.ticks.entry(s.clone()).or_default() += 1;
        }
        let mut ids: Vec<_> = self.pending.keys().cloned().collect();
        ids.sort();
        for id in ids {
            let Some(p) = self.pending.get_mut(&id) else {
                continue;
            };
            let Some(c) = candles.get(&p.req.symbol) else {
                continue;
            };
            let tick = *self.ticks.get(&p.req.symbol).unwrap_or(&0);
            if tick < p.due {
                continue;
            }
            if !p.acked {
                self.events.push(OrderEvent::Acked { id: id.clone() });
                p.acked = true;
            }
            let cap = self
                .partial_fills_pct
                .map(|pct| (c.volume * pct / 100.0).floor() as u64)
                .unwrap_or(p.remaining);
            let qty = p.remaining.min(cap);
            if qty == 0 {
                p.due = tick + 1;
                continue;
            }
            let raw_price = if p.req.side == Side::Buy {
                c.open * (1.0 + self.slippage_bps / 10000.0)
            } else {
                c.open * (1.0 - self.slippage_bps / 10000.0)
            };
            let tick_paise = self
                .tick_sizes_paise
                .get(&p.req.symbol)
                .copied()
                .unwrap_or(self.tick_size_paise)
                .max(1);
            let tick_units = raw_price * 100.0 / tick_paise as f64;
            let nearest = tick_units.round();
            let units = if (tick_units - nearest).abs() <= 1e-10 {
                nearest
            } else if p.req.side == Side::Buy {
                tick_units.ceil()
            } else {
                tick_units.floor()
            };
            let price_paise = (units as i64).saturating_mul(tick_paise);
            let price = crate::trader::paise_to_rupees(price_paise);
            p.notional_filled_paise += qty as i128 * price_paise as i128;
            let target_fee_paise = if self.fees.bps == 0.0
                && self.fees.flat_per_order == 0.0
                && self.fees.min_per_order == 0.0
            {
                0i64
            } else {
                // Convert the configured rate to integer hundredths of a bp
                // once; all notional multiplication and tie decisions below
                // are integer-only (denominator: 10,000 * 100).
                let bps_hundredths = round_half_even_f64(self.fees.bps * 100.0) as i128;
                let variable = round_half_even_i128(
                    p.notional_filled_paise * bps_hundredths,
                    10_000i128 * 100,
                );
                let flat = crate::trader::rupees_to_paise(self.fees.flat_per_order) as i128;
                let minimum = crate::trader::rupees_to_paise(self.fees.min_per_order) as i128;
                (variable + flat).max(minimum) as i64
            };
            let fees_paise = (target_fee_paise - p.fees_charged_paise).max(0);
            p.fees_charged_paise = target_fee_paise;
            let fees = crate::trader::paise_to_rupees(fees_paise);
            p.remaining -= qty;
            let done = p.remaining == 0;
            if done {
                self.events.push(OrderEvent::Filled {
                    id: id.clone(),
                    qty,
                    price,
                    fees,
                });
            } else {
                self.events.push(OrderEvent::PartialFill {
                    id: id.clone(),
                    qty,
                    price,
                    fees,
                });
                p.due = tick + 1;
            }
            if done {
                self.pending.remove(&id);
            }
        }
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

impl ExecutionPort for SimBroker {
    fn submit(&mut self, r: &OrderRequest) -> Result<(), PortError> {
        if r.qty == 0
            || !self.symbols.contains(&r.symbol)
            || self.seen_ids.contains(&r.client_order_id)
        {
            return Err(PortError::Message("invalid or duplicate order".into()));
        }
        let tick = *self.ticks.get(&r.symbol).unwrap_or(&0);
        self.pending.insert(
            r.client_order_id.clone(),
            PendingOrder {
                req: r.clone(),
                remaining: r.qty,
                due: tick + self.latency.max(1),
                acked: false,
                notional_filled_paise: 0,
                fees_charged_paise: 0,
            },
        );
        self.seen_ids.insert(r.client_order_id.clone());
        Ok(())
    }

    fn cancel(&mut self, id: OrderId) -> Result<(), PortError> {
        if self.pending.remove(&id).is_some() {
            self.events.push(OrderEvent::Cancelled { id });
            Ok(())
        } else {
            Err(PortError::UnknownOrder(id))
        }
    }

    fn drain_events(&mut self) -> Vec<OrderEvent> {
        self.events.sort_by(|a, b| event_id(a).cmp(event_id(b)));
        std::mem::take(&mut self.events)
    }
}

fn event_id(e: &OrderEvent) -> &str {
    match e {
        OrderEvent::Acked { id }
        | OrderEvent::PartialFill { id, .. }
        | OrderEvent::Filled { id, .. }
        | OrderEvent::Rejected { id, .. }
        | OrderEvent::Cancelled { id }
        | OrderEvent::Expired { id } => id,
    }
}
