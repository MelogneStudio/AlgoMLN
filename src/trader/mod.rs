//! Portfolio-wide synchronous order decisions and fill-driven accounting.

pub mod math;
pub mod port;
pub mod types;

#[cfg(test)]
mod tests;

pub use math::{paise_to_rupees, rupees_to_paise};
pub use port::{ExecutionPort, ScriptedPort};
pub use types::{
    FrameOutput, InstrumentRule, Mode, OrderEvent, OrderId, OrderReason, OrderRequest, OrderState,
    PortError, Position, Side, TraderConfig, TraderError, TraderEvent,
};

use crate::{
    ast::{Action, TradeAmount},
    engine::{LevelFrame, Program, RuleId, Symbol},
    indicators::Candle,
};
use math::{decimal_rupees_to_paise, decimal_scaled, round_half_even_ratio};
use std::collections::{BTreeMap, BTreeSet};
use types::{event_id, Pending};

#[derive(Debug)]
pub struct Trader {
    program: Program,
    cfg: TraderConfig,
    prev_levels: BTreeMap<(Symbol, RuleId), bool>,
    positions: BTreeMap<Symbol, Position>,
    orders: BTreeMap<OrderId, Pending>,
    completed_orders: BTreeMap<OrderId, OrderState>,
    last_prices: BTreeMap<Symbol, (f64, i64)>,
    cash_paise: i64,
    initial_cash_paise: i64,
    realized_pnl_today_paise: i64,
    realized_loss_today_paise: i64,
    accepted_buys_today: u64,
    next_id: u64,
    daily_loss_reported: bool,
}

impl Trader {
    pub fn new(program: &Program, cfg: TraderConfig) -> Result<Self, TraderError> {
        if program.risk().max_daily_loss.is_some()
            && (!cfg.initial_cash.is_finite() || cfg.initial_cash <= 0.0)
        {
            return Err(TraderError::InitialCashRequired);
        }
        let cash_paise = rupees_to_paise(cfg.initial_cash);
        Ok(Self {
            program: program.clone(),
            cfg,
            prev_levels: BTreeMap::new(),
            positions: BTreeMap::new(),
            orders: BTreeMap::new(),
            completed_orders: BTreeMap::new(),
            last_prices: BTreeMap::new(),
            cash_paise,
            initial_cash_paise: cash_paise,
            realized_pnl_today_paise: 0,
            realized_loss_today_paise: 0,
            accepted_buys_today: 0,
            next_id: 1,
            daily_loss_reported: false,
        })
    }

    pub fn start_day(&mut self, _day: i64) {
        self.realized_pnl_today_paise = 0;
        self.realized_loss_today_paise = 0;
        self.accepted_buys_today = 0;
        self.daily_loss_reported = false;
    }

    pub fn cash(&self) -> f64 {
        paise_to_rupees(self.cash_paise)
    }

    pub fn cash_paise(&self) -> i64 {
        self.cash_paise
    }

    pub fn realized_pnl_today(&self) -> f64 {
        paise_to_rupees(self.realized_pnl_today_paise)
    }

    pub fn realized_pnl_today_paise(&self) -> i64 {
        self.realized_pnl_today_paise
    }

    pub fn realized_loss_today(&self) -> f64 {
        paise_to_rupees(self.realized_loss_today_paise)
    }

    pub fn realized_loss_today_paise(&self) -> i64 {
        self.realized_loss_today_paise
    }

    pub fn accepted_buys_today(&self) -> u64 {
        self.accepted_buys_today
    }

    pub fn pending_order_count(&self) -> usize {
        self.orders.values().filter(|o| o.live()).count()
    }

    pub fn reserved_cash(&self) -> f64 {
        paise_to_rupees(self.reserved_cash_inner())
    }

    pub fn reserved_cash_paise(&self) -> i64 {
        self.reserved_cash_inner()
    }

    pub fn position(&self, s: &str) -> Option<&Position> {
        self.positions.get(s)
    }

    pub fn order_state(&self, id: &str) -> Option<OrderState> {
        self.orders
            .get(id)
            .map(|o| o.state)
            .or_else(|| self.completed_orders.get(id).copied())
    }

    pub fn on_order_event(&mut self, ev: OrderEvent) -> Vec<TraderEvent> {
        let id = event_id(&ev).to_owned();
        let mut events = Vec::new();
        let mut fill = None;
        {
            let Some(o) = self.orders.get_mut(&id) else {
                return events;
            };
            if !o.live() {
                return events;
            }
            match ev {
                OrderEvent::Acked { .. } => {
                    if o.state == OrderState::Submitted {
                        o.state = OrderState::Acked;
                        if o.req.side == Side::Buy {
                            self.accepted_buys_today += 1
                        }
                    }
                }
                OrderEvent::PartialFill {
                    qty, price, fees, ..
                }
                | OrderEvent::Filled {
                    qty, price, fees, ..
                } => {
                    if !matches!(
                        o.state,
                        OrderState::Submitted | OrderState::Acked | OrderState::PartialFill
                    ) {
                        return events;
                    }
                    if o.state == OrderState::Submitted && o.req.side == Side::Buy {
                        self.accepted_buys_today += 1;
                    }
                    if !price.is_finite() || price <= 0.0 || !fees.is_finite() || fees < 0.0 {
                        return events;
                    }
                    let remaining_before = o.remaining();
                    let q = qty.min(remaining_before);
                    if q > 0 {
                        o.filled += q;
                        let booked_price = paise_to_rupees(rupees_to_paise(price));
                        let booked_fees = paise_to_rupees(rupees_to_paise(fees));
                        fill = Some((
                            o.req.symbol.clone(),
                            o.req.side,
                            q,
                            booked_price,
                            booked_fees,
                        ));
                        events.push(TraderEvent::Fill {
                            id: id.clone(),
                            qty: q,
                            price: booked_price,
                            fees: booked_fees,
                        })
                    }
                    if matches!(ev, OrderEvent::Filled { .. }) || o.remaining() == 0 {
                        debug_assert!(
                            !matches!(ev, OrderEvent::Filled { .. }) || qty >= remaining_before,
                            "terminal Filled booked {q} of {remaining_before} remaining for {}; \
                             a broker adapter must report incremental fills or handle early close",
                            o.req.client_order_id
                        );
                        o.state = OrderState::Filled
                    } else {
                        o.state = OrderState::PartialFill
                    }
                }
                OrderEvent::Rejected { reason, .. } => {
                    o.state = OrderState::Rejected;
                    events.push(TraderEvent::OrderRejected {
                        id: id.clone(),
                        reason,
                    })
                }
                OrderEvent::Cancelled { .. } => o.state = OrderState::Cancelled,
                OrderEvent::Expired { .. } => o.state = OrderState::Expired,
            }
        }
        if let Some((s, side, q, p, fees)) = fill {
            self.apply_fill(&s, side, q, rupees_to_paise(p), rupees_to_paise(fees))
        }
        if self.orders.get(&id).is_some_and(|o| !o.live()) {
            events.push(TraderEvent::OrderClosed { id: id.clone() });
            if let Some(o) = self.orders.remove(&id) {
                self.completed_orders.insert(id, o.state);
            }
        }
        events
    }

    fn apply_fill(&mut self, s: &str, side: Side, q: u64, p: i64, fees: i64) {
        let pos = self.positions.entry(s.to_owned()).or_default();
        let q_i = q.min(i64::MAX as u64) as i64;
        match side {
            Side::Buy => {
                let old = pos.filled_qty.max(0);
                let total = old.saturating_add(q_i);
                let old_cost = (old as i128) * (pos.avg_entry_paise as i128);
                let fill_cost = (q_i as i128) * (p as i128) + fees as i128;
                pos.avg_entry_paise =
                    round_half_even_ratio(old_cost + fill_cost, total.max(1) as i128)
                        .clamp(i64::MIN as i128, i64::MAX as i128) as i64;
                pos.filled_qty = total;
                self.cash_paise = self
                    .cash_paise
                    .saturating_sub(q_i.saturating_mul(p).saturating_add(fees));
            }
            Side::Sell => {
                let closed = q.min(pos.filled_qty.max(0) as u64);
                let closed_i = closed.min(i64::MAX as u64) as i64;
                let realized = (p.saturating_sub(pos.avg_entry_paise))
                    .saturating_mul(closed_i)
                    .saturating_sub(fees);
                self.realized_pnl_today_paise =
                    self.realized_pnl_today_paise.saturating_add(realized);
                self.realized_loss_today_paise = self
                    .realized_loss_today_paise
                    .saturating_add(realized.saturating_neg().max(0));
                pos.filled_qty = pos
                    .filled_qty
                    .saturating_sub(q.min(i64::MAX as u64) as i64)
                    .max(0);
                if pos.filled_qty == 0 {
                    pos.avg_entry_paise = 0;
                }
                self.cash_paise = self
                    .cash_paise
                    .saturating_add(q_i.saturating_mul(p).saturating_sub(fees));
            }
        }
    }

    pub fn on_port_events(
        &mut self,
        events: impl IntoIterator<Item = OrderEvent>,
    ) -> Vec<TraderEvent> {
        events
            .into_iter()
            .flat_map(|e| self.on_order_event(e))
            .collect()
    }

    pub fn on_frame(&mut self, f: &LevelFrame, c: &Candle, mode: Mode) -> FrameOutput {
        let mut out = FrameOutput::default();
        let mut fires = Vec::new();
        let mut levels = BTreeMap::new();
        for (id, &level) in &f.entry {
            levels.insert(id.clone(), (level, true));
        }
        for (id, &level) in &f.exit {
            levels.insert(id.clone(), (level, false));
        }
        for (id, (level, is_entry)) in levels {
            let prev = self
                .prev_levels
                .insert((f.symbol.clone(), id.clone()), level)
                .unwrap_or(false);
            if level && !prev {
                fires.push((id, is_entry))
            }
        }
        if mode == Mode::Observe {
            return out;
        }
        self.last_prices
            .insert(f.symbol.clone(), (c.close, f.candle_ts));
        let loss_breached = self.daily_loss_breached();
        if loss_breached {
            let report_daily_loss = !self.daily_loss_reported;
            self.daily_loss_reported = true;
            for o in self.orders.values_mut() {
                if o.live() && !o.cancel_requested && o.req.side == Side::Buy && o.remaining() > 0 {
                    o.cancel_requested = true;
                    out.cancels.push(o.req.client_order_id.clone());
                }
            }
            let open: Vec<_> = self
                .positions
                .iter()
                .filter(|(_, position)| position.filled_qty > 0)
                .map(|(symbol, position)| (symbol.clone(), position.filled_qty as u64))
                .collect();
            for (symbol, qty) in open {
                if !self.has_pending_exit(&symbol) {
                    if let Some((price, decided_at)) = self.last_prices.get(&symbol).copied() {
                        self.emit_at(
                            &mut out,
                            &symbol,
                            Side::Sell,
                            qty,
                            OrderReason::DailyLossFlatten,
                            price,
                            decided_at,
                        );
                    }
                }
            }
            for (id, is_entry) in fires {
                if is_entry {
                    out.events.push(TraderEvent::RiskBlocked {
                        symbol: f.symbol.clone(),
                        rule_id: id,
                        limit: "MAX_DAILY_LOSS",
                    });
                }
            }
            if report_daily_loss {
                out.events.push(TraderEvent::DailyLossBreached);
            }
            return out;
        }
        let mut full_exit = false;
        for (id, is_entry) in fires {
            let Some(rule) = self.program.rules().iter().find(|r| r.id == id) else {
                continue;
            };
            let side = if is_entry { Side::Buy } else { Side::Sell };
            let net = self.net_position(&f.symbol);
            let raw = match &rule.action {
                Action::Buy(a) | Action::Sell(a) => self.resolve_size(a, c.close, &f.symbol),
                Action::SellAll => Some(net.max(0) as u64),
            }
            .unwrap_or(0);
            let qty = if is_entry {
                raw
            } else {
                raw.min(net.max(0) as u64)
            };
            if qty == 0 {
                out.events.push(TraderEvent::RuleSkipped {
                    symbol: f.symbol.clone(),
                    rule_id: id,
                });
                continue;
            }
            if is_entry {
                if let Some(limit) = self.blocking_limit(&f.symbol, qty, c.close) {
                    out.events.push(TraderEvent::RiskBlocked {
                        symbol: f.symbol.clone(),
                        rule_id: id,
                        limit,
                    });
                    continue;
                }
            }
            if !is_entry && qty as i64 >= net {
                full_exit = true
            }
            self.emit_at(
                &mut out,
                &f.symbol,
                side,
                qty,
                OrderReason::Rule(id),
                c.close,
                f.candle_ts,
            )
        }
        if !full_exit && !self.has_pending_exit(&f.symbol) {
            if let Some(pos) = self.positions.get(&f.symbol).filter(|p| p.filled_qty > 0) {
                let stop = self
                    .program
                    .stop_loss()
                    .and_then(|v| decimal_scaled(&v.spelling, 100))
                    .is_some_and(|bp| {
                        rupees_to_paise(c.close)
                            <= round_half_even_ratio(
                                pos.avg_entry_paise as i128 * (10_000 - bp),
                                10_000,
                            ) as i64
                    });
                let take = self
                    .program
                    .take_profit()
                    .and_then(|v| decimal_scaled(&v.spelling, 100))
                    .is_some_and(|bp| {
                        rupees_to_paise(c.close)
                            >= round_half_even_ratio(
                                pos.avg_entry_paise as i128 * (10_000 + bp),
                                10_000,
                            ) as i64
                    });
                if stop || take {
                    self.emit_at(
                        &mut out,
                        &f.symbol,
                        Side::Sell,
                        pos.filled_qty as u64,
                        if stop {
                            OrderReason::StopLoss
                        } else {
                            OrderReason::TakeProfit
                        },
                        c.close,
                        f.candle_ts,
                    )
                }
            }
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_at(
        &mut self,
        out: &mut FrameOutput,
        s: &str,
        side: Side,
        qty: u64,
        reason: OrderReason,
        ref_price: f64,
        decided_at: i64,
    ) {
        let req = OrderRequest {
            client_order_id: format!("trader-{}", self.next_id),
            symbol: s.to_owned(),
            side,
            qty,
            reason,
            ref_price,
            decided_at,
        };
        self.next_id += 1;
        self.orders.insert(
            req.client_order_id.clone(),
            Pending {
                req: req.clone(),
                filled: 0,
                state: OrderState::Submitted,
                cancel_requested: false,
            },
        );
        out.events.push(TraderEvent::OrderEmitted(req.clone()));
        out.orders.push(req)
    }

    fn has_pending_exit(&self, s: &str) -> bool {
        self.orders
            .values()
            .any(|o| o.live() && o.req.symbol == s && o.req.side == Side::Sell)
    }

    pub fn net_position(&self, s: &str) -> i64 {
        let filled = self.positions.get(s).map_or(0, |p| p.filled_qty);
        let reserved = self
            .orders
            .values()
            .filter(|o| o.live() && o.req.symbol == s && o.req.side == Side::Sell)
            .map(|o| o.remaining() as i64)
            .sum::<i64>();
        (filled - reserved).max(0)
    }

    fn reserved_cash_inner(&self) -> i64 {
        self.orders
            .values()
            .filter(|o| o.live() && o.req.side == Side::Buy)
            .map(|o| {
                (o.remaining().min(i64::MAX as u64) as i64)
                    .saturating_mul(rupees_to_paise(o.req.ref_price))
            })
            .fold(0i64, i64::saturating_add)
    }

    fn available_cash(&self) -> i64 {
        self.cash_paise
            .saturating_sub(self.reserved_cash_inner())
            .max(0)
    }

    fn pending_new(&self) -> BTreeSet<Symbol> {
        self.orders
            .values()
            .filter(|o| {
                o.live()
                    && o.req.side == Side::Buy
                    && self
                        .positions
                        .get(&o.req.symbol)
                        .map_or(true, |p| p.filled_qty == 0)
            })
            .map(|o| o.req.symbol.clone())
            .collect()
    }

    fn blocking_limit(&self, _s: &str, q: u64, p: f64) -> Option<&'static str> {
        let mut symbols: BTreeSet<Symbol> = self
            .positions
            .iter()
            .filter(|(_, x)| x.filled_qty > 0)
            .map(|(s, _)| s.clone())
            .collect();
        symbols.extend(self.pending_new());
        if self
            .program
            .risk()
            .max_positions
            .is_some_and(|m| symbols.len() as u64 >= m)
        {
            return Some("MAX_POSITIONS");
        }
        let unacked = self
            .orders
            .values()
            .filter(|o| o.live() && o.req.side == Side::Buy && o.state == OrderState::Submitted)
            .count() as u64;
        if self
            .program
            .risk()
            .max_orders
            .is_some_and(|m| self.accepted_buys_today + unacked >= m)
        {
            return Some("MAX_ORDERS");
        }
        if self.daily_loss_breached() {
            return Some("MAX_DAILY_LOSS");
        }
        if (self.available_cash() as i128) < q as i128 * rupees_to_paise(p) as i128 {
            return Some("CASH");
        }
        None
    }

    fn daily_loss_breached(&self) -> bool {
        self.program
            .risk()
            .max_daily_loss
            .as_ref()
            .is_some_and(|limit| {
                self.initial_cash_paise > 0
                    && self.realized_loss_today_paise as i128 * 10_000
                        >= decimal_scaled(&limit.spelling, 100).unwrap_or(0)
                            * self.initial_cash_paise as i128
            })
    }

    pub fn resolve_size(&self, a: &TradeAmount, p: f64, s: &str) -> Option<u64> {
        if !p.is_finite() || p <= 0.0 {
            return None;
        }
        let raw = match a {
            TradeAmount::Quantity(n) => *n,
            TradeAmount::Worth(x) => {
                let worth_paise = decimal_rupees_to_paise(&x.spelling)?;
                let ref_paise = rupees_to_paise(p);
                if ref_paise <= 0 {
                    0
                } else {
                    (worth_paise.max(0) as i128 / ref_paise as i128) as u64
                }
            }
            TradeAmount::BuyCapitalPercentage(x) => {
                let pct_bp = decimal_scaled(&x.spelling, 100).unwrap_or(0);
                let ref_paise = rupees_to_paise(p);
                if ref_paise <= 0 {
                    0
                } else {
                    (((self.available_cash() as i128 * pct_bp) / (ref_paise as i128 * 10_000))
                        .max(0)) as u64
                }
            }
            TradeAmount::SellPositionPercentage(x) => {
                (((self.net_position(s) as i128 * decimal_scaled(&x.spelling, 100).unwrap_or(0))
                    / 10_000)
                    .max(0)) as u64
            }
        };
        let r = self
            .cfg
            .instrument_rules
            .get(s)
            .copied()
            .unwrap_or_default();
        let lot = r.lot_size.max(1);
        let q = raw / lot * lot;
        (q >= r.min_qty).then_some(q)
    }
}
