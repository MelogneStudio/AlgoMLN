//! Portfolio-wide synchronous order decisions and fill-driven accounting.
use crate::{
    ast::{Action, TradeAmount},
    engine::{LevelFrame, Program, RuleId, Symbol},
    indicators::Candle,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub type OrderId = String;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Act,
    Observe,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Buy,
    Sell,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderReason {
    Rule(RuleId),
    StopLoss,
    TakeProfit,
    DailyLossFlatten,
}
#[derive(Clone, Debug, PartialEq)]
pub struct OrderRequest {
    pub client_order_id: OrderId,
    pub symbol: Symbol,
    pub side: Side,
    pub qty: u64,
    pub reason: OrderReason,
    pub ref_price: f64,
    pub decided_at: i64,
}
#[derive(Clone, Debug, PartialEq)]
pub enum OrderEvent {
    Acked {
        id: OrderId,
    },
    PartialFill {
        id: OrderId,
        qty: u64,
        price: f64,
        fees: f64,
    },
    Filled {
        id: OrderId,
        qty: u64,
        price: f64,
        fees: f64,
    },
    Rejected {
        id: OrderId,
        reason: String,
    },
    Cancelled {
        id: OrderId,
    },
    Expired {
        id: OrderId,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PortError {
    Message(String),
}
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstrumentRule {
    pub lot_size: u64,
    pub min_qty: u64,
}
impl Default for InstrumentRule {
    fn default() -> Self {
        Self {
            lot_size: 1,
            min_qty: 1,
        }
    }
}
#[derive(Clone, Debug)]
pub struct TraderConfig {
    pub initial_cash: f64,
    pub instrument_rules: BTreeMap<Symbol, InstrumentRule>,
}
impl Default for TraderConfig {
    fn default() -> Self {
        Self {
            initial_cash: 0.0,
            instrument_rules: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraderError {
    InitialCashRequired,
}
#[derive(Clone, Debug, PartialEq)]
pub enum TraderEvent {
    RuleSkipped {
        symbol: Symbol,
        rule_id: RuleId,
    },
    RiskBlocked {
        symbol: Symbol,
        rule_id: RuleId,
        limit: &'static str,
    },
    OrderEmitted(OrderRequest),
    Fill {
        id: OrderId,
        qty: u64,
        price: f64,
        fees: f64,
    },
    OrderRejected {
        id: OrderId,
        reason: String,
    },
    OrderClosed {
        id: OrderId,
    },
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameOutput {
    pub orders: Vec<OrderRequest>,
    /// Broker order IDs the adapter should cancel after this frame.
    pub cancels: Vec<OrderId>,
    pub events: Vec<TraderEvent>,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Position {
    pub filled_qty: i64,
    pub avg_entry: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderState {
    Submitted,
    Acked,
    PartialFill,
    Filled,
    Rejected,
    Cancelled,
    Expired,
}
#[derive(Clone, Debug)]
struct Pending {
    req: OrderRequest,
    filled: u64,
    state: OrderState,
}
impl Pending {
    fn live(&self) -> bool {
        matches!(
            self.state,
            OrderState::Submitted | OrderState::Acked | OrderState::PartialFill
        )
    }
    fn remaining(&self) -> u64 {
        self.req.qty.saturating_sub(self.filled)
    }
}

#[derive(Debug)]
pub struct Trader {
    program: Program,
    cfg: TraderConfig,
    prev_levels: BTreeMap<(Symbol, RuleId), bool>,
    positions: BTreeMap<Symbol, Position>,
    orders: BTreeMap<OrderId, Pending>,
    last_prices: BTreeMap<Symbol, (f64, i64)>,
    cash: f64,
    realized_pnl_today: f64,
    realized_loss_today: f64,
    accepted_buys_today: u64,
    next_id: u64,
}
impl Trader {
    pub fn new(program: &Program, cfg: TraderConfig) -> Result<Self, TraderError> {
        if program.risk().max_daily_loss.is_some()
            && (!cfg.initial_cash.is_finite() || cfg.initial_cash <= 0.0)
        {
            return Err(TraderError::InitialCashRequired);
        }
        let cash = cfg.initial_cash;
        Ok(Self {
            program: program.clone(),
            cfg,
            prev_levels: BTreeMap::new(),
            positions: BTreeMap::new(),
            orders: BTreeMap::new(),
            last_prices: BTreeMap::new(),
            cash,
            realized_pnl_today: 0.0,
            realized_loss_today: 0.0,
            accepted_buys_today: 0,
            next_id: 1,
        })
    }
    pub fn start_day(&mut self, _day: i64) {
        self.realized_pnl_today = 0.0;
        self.realized_loss_today = 0.0;
        self.accepted_buys_today = 0
    }
    pub fn cash(&self) -> f64 {
        self.cash
    }
    pub fn realized_pnl_today(&self) -> f64 {
        self.realized_pnl_today
    }
    pub fn realized_loss_today(&self) -> f64 {
        self.realized_loss_today
    }
    pub fn accepted_buys_today(&self) -> u64 {
        self.accepted_buys_today
    }
    pub fn position(&self, s: &str) -> Option<&Position> {
        self.positions.get(s)
    }
    pub fn order_state(&self, id: &str) -> Option<OrderState> {
        self.orders.get(id).map(|o| o.state)
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
                    if !matches!(o.state, OrderState::Acked | OrderState::PartialFill) {
                        return events;
                    }
                    if !price.is_finite() || price <= 0.0 || !fees.is_finite() || fees < 0.0 {
                        return events;
                    }
                    let q = qty.min(o.remaining());
                    if q > 0 {
                        o.filled += q;
                        fill = Some((o.req.symbol.clone(), o.req.side, q, price, fees));
                        events.push(TraderEvent::Fill {
                            id: id.clone(),
                            qty: q,
                            price,
                            fees,
                        })
                    }
                    if matches!(ev, OrderEvent::Filled { .. }) || o.remaining() == 0 {
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
            self.apply_fill(&s, side, q, p, fees)
        }
        if self.orders.get(&id).is_some_and(|o| !o.live()) {
            events.push(TraderEvent::OrderClosed { id })
        }
        events
    }
    fn apply_fill(&mut self, s: &str, side: Side, q: u64, p: f64, fees: f64) {
        let pos = self.positions.entry(s.to_owned()).or_default();
        let qf = q as f64;
        match side {
            Side::Buy => {
                let old = pos.filled_qty.max(0) as f64;
                let total = old + qf;
                pos.avg_entry = (old * pos.avg_entry + qf * p + fees) / total;
                pos.filled_qty = pos.filled_qty.saturating_add(q.min(i64::MAX as u64) as i64);
                self.cash -= qf * p + fees
            }
            Side::Sell => {
                let closed = q.min(pos.filled_qty.max(0) as u64);
                let realized = (p - pos.avg_entry) * closed as f64 - fees;
                self.realized_pnl_today += realized;
                self.realized_loss_today += (-realized).max(0.0);
                pos.filled_qty = pos
                    .filled_qty
                    .saturating_sub(q.min(i64::MAX as u64) as i64)
                    .max(0);
                if pos.filled_qty == 0 {
                    pos.avg_entry = 0.0
                }
                self.cash += qf * p - fees
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
            out.cancels.extend(
                self.orders
                    .values()
                    .filter(|o| o.live() && o.req.side == Side::Buy && o.remaining() > 0)
                    .map(|o| o.req.client_order_id.clone()),
            );
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
            // Entries still flow through the risk gate for an explicit block
            // event. Rule exits and automatic exits must not compete with the
            // portfolio flatten order.
            for (id, is_entry) in fires {
                if is_entry {
                    out.events.push(TraderEvent::RiskBlocked {
                        symbol: f.symbol.clone(),
                        rule_id: id,
                        limit: "MAX_DAILY_LOSS",
                    });
                }
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
                    .and_then(|v| v.spelling.parse::<f64>().ok())
                    .is_some_and(|x| c.close <= pos.avg_entry * (1.0 - x / 100.0));
                let take = self
                    .program
                    .take_profit()
                    .and_then(|v| v.spelling.parse::<f64>().ok())
                    .is_some_and(|x| c.close >= pos.avg_entry * (1.0 + x / 100.0));
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
    fn net_position(&self, s: &str) -> i64 {
        let filled = self.positions.get(s).map_or(0, |p| p.filled_qty);
        let reserved = self
            .orders
            .values()
            .filter(|o| o.live() && o.req.symbol == s && o.req.side == Side::Sell)
            .map(|o| o.remaining() as i64)
            .sum::<i64>();
        (filled - reserved).max(0)
    }
    fn reserved_cash(&self) -> f64 {
        self.orders
            .values()
            .filter(|o| o.live() && o.req.side == Side::Buy)
            .map(|o| o.remaining() as f64 * o.req.ref_price)
            .sum()
    }
    fn available_cash(&self) -> f64 {
        (self.cash - self.reserved_cash()).max(0.0)
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
                        .is_none_or(|p| p.filled_qty == 0)
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
        if self.available_cash() + f64::EPSILON < q as f64 * p {
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
                self.cfg.initial_cash > 0.0
                    && self.realized_loss_today / self.cfg.initial_cash
                        >= limit.spelling.parse::<f64>().unwrap_or(0.0) / 100.0
            })
    }
    fn resolve_size(&self, a: &TradeAmount, p: f64, s: &str) -> Option<u64> {
        if !p.is_finite() || p <= 0.0 {
            return None;
        }
        let raw = match a {
            TradeAmount::Quantity(n) => *n,
            TradeAmount::Worth(x) => (x.spelling.parse::<f64>().ok()? / p).floor() as u64,
            TradeAmount::BuyCapitalPercentage(x) => {
                (self.available_cash() * x.spelling.parse::<f64>().ok()? / 100.0 / p).floor() as u64
            }
            TradeAmount::SellPositionPercentage(x) => {
                ((self.net_position(s) as f64 * x.spelling.parse::<f64>().ok()? / 100.0).floor())
                    as u64
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::LevelFrame;

    fn candle(ts: i64, close: f64) -> Candle {
        Candle {
            timestamp: ts,
            open: close,
            high: close,
            low: close,
            close,
            volume: 100.0,
        }
    }
    fn program(src: &str) -> Program {
        crate::compile(src).unwrap()
    }
    fn frame(ts: i64, entry: &[(&str, bool)], exit: &[(&str, bool)]) -> LevelFrame {
        LevelFrame {
            symbol: "ABC".into(),
            candle_ts: ts,
            warm: true,
            entry: entry.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
            exit: exit.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
        }
    }
    fn named_frame(
        symbol: &str,
        ts: i64,
        entry: &[(&str, bool)],
        exit: &[(&str, bool)],
    ) -> LevelFrame {
        let mut f = frame(ts, entry, exit);
        f.symbol = symbol.into();
        f
    }
    fn trader(src: &str, cash: f64) -> Trader {
        Trader::new(
            &program(src),
            TraderConfig {
                initial_cash: cash,
                instrument_rules: BTreeMap::new(),
            },
        )
        .unwrap()
    }
    fn fill_buy(t: &mut Trader, id: &str, qty: u64, price: f64) {
        t.on_order_event(OrderEvent::Acked { id: id.into() });
        t.on_order_event(OrderEvent::Filled {
            id: id.into(),
            qty,
            price,
            fees: 0.0,
        });
    }

    #[test]
    fn observe_updates_edges_and_emits_nothing() {
        let mut t = trader("TRADE_IN ABC\nWHEN close > 1\nBUY 2\n", 100.0);
        let f = frame(1, &[("rule_1", true)], &[]);
        assert!(t
            .on_frame(&f, &candle(1, 10.0), Mode::Observe)
            .orders
            .is_empty());
        assert!(t
            .on_frame(
                &frame(2, &[("rule_1", true)], &[]),
                &candle(2, 10.0),
                Mode::Act
            )
            .orders
            .is_empty());
        assert_eq!(
            t.on_frame(
                &frame(3, &[("rule_1", false)], &[]),
                &candle(3, 10.0),
                Mode::Act
            )
            .orders
            .len(),
            0
        );
        assert_eq!(
            t.on_frame(
                &frame(4, &[("rule_1", true)], &[]),
                &candle(4, 10.0),
                Mode::Act
            )
            .orders[0]
                .qty,
            2
        );
    }

    #[test]
    fn pending_cash_and_unacked_orders_reserve_capacity_until_events() {
        let mut t = trader(
            "TRADE_IN ABC\nRISK MAX_ORDERS 1\nWHEN close > 1\nBUY 5\n",
            50.0,
        );
        let out = t.on_frame(
            &frame(1, &[("rule_1", true)], &[]),
            &candle(1, 10.0),
            Mode::Act,
        );
        assert_eq!(out.orders[0].qty, 5);
        assert_eq!(t.cash(), 50.0); // pending orders do not alter the ledger
        let blocked = t.on_frame(
            &frame(2, &[("rule_1", false)], &[]),
            &candle(2, 10.0),
            Mode::Act,
        );
        let again = t.on_frame(
            &frame(3, &[("rule_1", true)], &[]),
            &candle(3, 10.0),
            Mode::Act,
        );
        assert!(blocked.orders.is_empty());
        assert!(matches!(
            again.events[0],
            TraderEvent::RiskBlocked {
                limit: "MAX_ORDERS",
                ..
            }
        ));
        t.on_order_event(OrderEvent::Rejected {
            id: "trader-1".into(),
            reason: "no".into(),
        });
        let after = t.on_frame(
            &frame(4, &[("rule_1", false)], &[]),
            &candle(4, 10.0),
            Mode::Act,
        );
        let emitted = t.on_frame(
            &frame(5, &[("rule_1", true)], &[]),
            &candle(5, 10.0),
            Mode::Act,
        );
        assert!(after.orders.is_empty());
        assert_eq!(emitted.orders.len(), 1);
    }

    #[test]
    fn cash_is_reserved_across_orders_in_a_single_frame() {
        let mut t = trader(
            "TRADE_IN ABC\nWHEN close > 1\nBUY 4\nWHEN close > 1\nBUY 2\n",
            50.0,
        );
        let out = t.on_frame(
            &frame(1, &[("rule_1", true), ("rule_2", true)], &[]),
            &candle(1, 10.0),
            Mode::Act,
        );
        assert_eq!(out.orders.len(), 1);
        assert_eq!(out.orders[0].qty, 4);
        assert!(matches!(
            out.events[1],
            TraderEvent::RiskBlocked { limit: "CASH", .. }
        ));
    }

    #[test]
    fn partial_fills_change_ledger_and_pending_exit_guards_stop_loss() {
        let mut t = trader(
            "TRADE_IN ABC\nSTOP_LOSS 10%\nWHEN close > 1\nBUY 10\n",
            1000.0,
        );
        let buy = t
            .on_frame(
                &frame(1, &[("rule_1", true)], &[]),
                &candle(1, 10.0),
                Mode::Act,
            )
            .orders
            .remove(0);
        t.on_order_event(OrderEvent::Acked {
            id: buy.client_order_id.clone(),
        });
        t.on_order_event(OrderEvent::PartialFill {
            id: buy.client_order_id.clone(),
            qty: 4,
            price: 10.0,
            fees: 1.0,
        });
        assert_eq!(t.position("ABC").unwrap().filled_qty, 4);
        assert_eq!(t.cash(), 959.0);
        let exit = t.on_frame(
            &frame(2, &[("rule_1", false)], &[]),
            &candle(2, 8.0),
            Mode::Act,
        );
        assert_eq!(exit.orders.len(), 1);
        assert_eq!(exit.orders[0].qty, 4);
        let next = t.on_frame(
            &frame(3, &[("rule_1", false)], &[]),
            &candle(3, 7.0),
            Mode::Act,
        );
        assert!(next.orders.is_empty());
        t.on_order_event(OrderEvent::Rejected {
            id: exit.orders[0].client_order_id.clone(),
            reason: "rejected".into(),
        });
        let rearmed = t.on_frame(
            &frame(4, &[("rule_1", false)], &[]),
            &candle(4, 7.0),
            Mode::Act,
        );
        assert_eq!(rearmed.orders[0].reason, OrderReason::StopLoss);
    }

    #[test]
    fn max_daily_loss_requires_cash_and_blocks_at_threshold() {
        let p=program("TRADE_IN ABC\nRISK MAX_DAILY_LOSS 5%\nWHEN close > 1\nBUY 10\nWHEN close < 1\nSELL ALL\n");
        assert_eq!(
            Trader::new(&p, TraderConfig::default()).unwrap_err(),
            TraderError::InitialCashRequired
        );
        let mut t = Trader::new(
            &p,
            TraderConfig {
                initial_cash: 100.0,
                instrument_rules: BTreeMap::new(),
            },
        )
        .unwrap();
        let buy = t
            .on_frame(
                &frame(1, &[("rule_1", true)], &[("rule_2", false)]),
                &candle(1, 10.0),
                Mode::Act,
            )
            .orders
            .remove(0);
        fill_buy(&mut t, &buy.client_order_id, 10, 10.0);
        let sell = t
            .on_frame(
                &frame(2, &[("rule_1", false)], &[("rule_2", true)]),
                &candle(2, 9.0),
                Mode::Act,
            )
            .orders
            .remove(0);
        t.on_order_event(OrderEvent::Acked {
            id: sell.client_order_id.clone(),
        });
        t.on_order_event(OrderEvent::Filled {
            id: sell.client_order_id,
            qty: 10,
            price: 9.5,
            fees: 0.0,
        });
        assert_eq!(t.realized_pnl_today(), -5.0);
        assert_eq!(t.realized_loss_today(), 5.0);
        let blocked = t.on_frame(
            &frame(3, &[("rule_1", true)], &[("rule_2", false)]),
            &candle(3, 10.0),
            Mode::Act,
        );
        assert!(matches!(
            blocked.events[0],
            TraderEvent::RiskBlocked {
                limit: "MAX_DAILY_LOSS",
                ..
            }
        ));
        t.start_day(4);
        assert_eq!(t.realized_pnl_today(), 0.0);
        assert_eq!(t.realized_loss_today(), 0.0);
    }

    #[test]
    fn daily_loss_breach_cancels_entries_flattens_and_catches_late_fills() {
        let mut t = trader(
            "TRADE_IN ABC\nRISK MAX_DAILY_LOSS 1%\nWHEN close > 4\nBUY 10\nWHEN close > 4\nBUY 2\nWHEN close < 6\nSELL 1\n",
            100.0,
        );
        let entries = t.on_frame(
            &frame(
                1,
                &[("rule_1", true), ("rule_2", true)],
                &[("rule_3", false)],
            ),
            &candle(1, 5.0),
            Mode::Act,
        );
        assert_eq!(entries.orders.len(), 2);
        t.on_order_event(OrderEvent::Acked {
            id: entries.orders[0].client_order_id.clone(),
        });
        t.on_order_event(OrderEvent::Filled {
            id: entries.orders[0].client_order_id.clone(),
            qty: 10,
            price: 5.0,
            fees: 0.0,
        });

        let sell = t
            .on_frame(
                &frame(
                    2,
                    &[("rule_1", false), ("rule_2", false)],
                    &[("rule_3", true)],
                ),
                &candle(2, 4.0),
                Mode::Act,
            )
            .orders
            .remove(0);
        t.on_order_event(OrderEvent::Acked {
            id: sell.client_order_id.clone(),
        });
        t.on_order_event(OrderEvent::Filled {
            id: sell.client_order_id,
            qty: 1,
            price: 4.0,
            fees: 0.0,
        });
        assert_eq!(t.realized_pnl_today(), -1.0);

        let breach = t.on_frame(
            &frame(
                3,
                &[("rule_1", true), ("rule_2", false)],
                &[("rule_3", false)],
            ),
            &candle(3, 4.0),
            Mode::Act,
        );
        assert_eq!(breach.cancels, vec!["trader-2"]);
        assert_eq!(breach.orders.len(), 1);
        assert_eq!(breach.orders[0].qty, 9);
        assert_eq!(breach.orders[0].reason, OrderReason::DailyLossFlatten);
        assert!(breach.events.iter().any(|e| matches!(
            e,
            TraderEvent::RiskBlocked {
                limit: "MAX_DAILY_LOSS",
                ..
            }
        )));

        // A fill racing the cancellation is booked; the still-active breach
        // causes another flatten after the first flatten completes.
        t.on_order_event(OrderEvent::Acked {
            id: "trader-2".into(),
        });
        t.on_order_event(OrderEvent::Filled {
            id: "trader-2".into(),
            qty: 2,
            price: 4.0,
            fees: 0.0,
        });
        t.on_order_event(OrderEvent::Acked {
            id: breach.orders[0].client_order_id.clone(),
        });
        t.on_order_event(OrderEvent::Filled {
            id: breach.orders[0].client_order_id.clone(),
            qty: 9,
            price: 4.0,
            fees: 0.0,
        });
        let second_flatten = t.on_frame(
            &frame(
                4,
                &[("rule_1", false), ("rule_2", false)],
                &[("rule_3", false)],
            ),
            &candle(4, 4.0),
            Mode::Act,
        );
        assert_eq!(
            second_flatten.orders[0].reason,
            OrderReason::DailyLossFlatten
        );
        assert_eq!(second_flatten.orders[0].qty, 2);

        t.start_day(5);
        let _ = t.on_frame(
            &frame(
                5,
                &[("rule_1", false), ("rule_2", false)],
                &[("rule_3", false)],
            ),
            &candle(5, 4.0),
            Mode::Act,
        );
        let next_day = t.on_frame(
            &frame(
                6,
                &[("rule_1", true), ("rule_2", false)],
                &[("rule_3", false)],
            ),
            &candle(6, 5.0),
            Mode::Act,
        );
        assert!(!next_day.orders.is_empty());
    }

    #[test]
    fn daily_loss_flattens_other_portfolio_symbols_at_their_latest_close() {
        let mut t = trader(
            "TRADE_IN ABC,XYZ\nRISK MAX_DAILY_LOSS 1%\nWHEN close > 4\nBUY 1\nWHEN close < 10\nSELL 1\n",
            100.0,
        );
        let abc_buy = t
            .on_frame(
                &named_frame("ABC", 1, &[("rule_1", true)], &[("rule_2", false)]),
                &candle(1, 10.0),
                Mode::Act,
            )
            .orders
            .remove(0);
        t.on_order_event(OrderEvent::Acked {
            id: abc_buy.client_order_id.clone(),
        });
        t.on_order_event(OrderEvent::Filled {
            id: abc_buy.client_order_id,
            qty: 1,
            price: 10.0,
            fees: 0.0,
        });
        let xyz_buy = t
            .on_frame(
                &named_frame("XYZ", 2, &[("rule_1", true)], &[("rule_2", false)]),
                &candle(2, 12.0),
                Mode::Act,
            )
            .orders
            .remove(0);
        t.on_order_event(OrderEvent::Acked {
            id: xyz_buy.client_order_id.clone(),
        });
        t.on_order_event(OrderEvent::Filled {
            id: xyz_buy.client_order_id,
            qty: 1,
            price: 12.0,
            fees: 0.0,
        });
        let abc_sell = t
            .on_frame(
                &named_frame("ABC", 3, &[("rule_1", false)], &[("rule_2", true)]),
                &candle(3, 9.0),
                Mode::Act,
            )
            .orders
            .remove(0);
        t.on_order_event(OrderEvent::Acked {
            id: abc_sell.client_order_id.clone(),
        });
        t.on_order_event(OrderEvent::Filled {
            id: abc_sell.client_order_id,
            qty: 1,
            price: 9.0,
            fees: 0.0,
        });

        let breached = t.on_frame(
            &named_frame("ABC", 4, &[("rule_1", false)], &[("rule_2", false)]),
            &candle(4, 8.0),
            Mode::Act,
        );
        assert_eq!(breached.orders.len(), 1);
        assert_eq!(breached.orders[0].symbol, "XYZ");
        assert_eq!(breached.orders[0].ref_price, 12.0);
        assert_eq!(breached.orders[0].reason, OrderReason::DailyLossFlatten);
    }

    #[test]
    fn threshold_remains_breached_after_later_profit_offsets_net_pnl() {
        let mut t = trader(
            "TRADE_IN ABC\nRISK MAX_DAILY_LOSS 5%\nWHEN close > 1\nBUY 10\nWHEN close < 1\nSELL 5\nWHEN close < 1\nSELL 5\n",
            100.0,
        );
        let buy = t
            .on_frame(
                &frame(
                    1,
                    &[("rule_1", true)],
                    &[("rule_2", false), ("rule_3", false)],
                ),
                &candle(1, 10.0),
                Mode::Act,
            )
            .orders
            .remove(0);
        fill_buy(&mut t, &buy.client_order_id, 10, 10.0);
        let exits = t.on_frame(
            &frame(
                2,
                &[("rule_1", false)],
                &[("rule_2", true), ("rule_3", true)],
            ),
            &candle(2, 9.0),
            Mode::Act,
        );
        assert_eq!(exits.orders.len(), 2);
        t.on_order_event(OrderEvent::Acked {
            id: exits.orders[0].client_order_id.clone(),
        });
        t.on_order_event(OrderEvent::Filled {
            id: exits.orders[0].client_order_id.clone(),
            qty: 5,
            price: 9.0,
            fees: 0.0,
        });
        t.on_order_event(OrderEvent::Acked {
            id: exits.orders[1].client_order_id.clone(),
        });
        t.on_order_event(OrderEvent::Filled {
            id: exits.orders[1].client_order_id.clone(),
            qty: 5,
            price: 11.0,
            fees: 0.0,
        });
        assert_eq!(t.realized_pnl_today(), 0.0);
        assert_eq!(t.realized_loss_today(), 5.0);
        let later = t.on_frame(
            &frame(
                3,
                &[("rule_1", true)],
                &[("rule_2", false), ("rule_3", false)],
            ),
            &candle(3, 10.0),
            Mode::Act,
        );
        assert!(later.orders.is_empty());
        assert!(later.events.iter().any(|event| matches!(
            event,
            TraderEvent::RiskBlocked {
                limit: "MAX_DAILY_LOSS",
                ..
            }
        )));
    }

    #[test]
    fn sizing_rounds_down_to_lot_and_sl_beats_tp() {
        let src = "TRADE_IN ABC\nSTOP_LOSS 10%\nTAKE_PROFIT 10%\nWHEN close > 1\nBUY 10\n";
        let mut t = Trader::new(
            &program(src),
            TraderConfig {
                initial_cash: 1000.0,
                instrument_rules: BTreeMap::from([(
                    "ABC".into(),
                    InstrumentRule {
                        lot_size: 3,
                        min_qty: 3,
                    },
                )]),
            },
        )
        .unwrap();
        let buy = t
            .on_frame(
                &frame(1, &[("rule_1", true)], &[]),
                &candle(1, 10.0),
                Mode::Act,
            )
            .orders
            .remove(0);
        assert_eq!(buy.qty, 9);
        fill_buy(&mut t, &buy.client_order_id, 9, 10.0);
        let exit = t.on_frame(
            &frame(2, &[("rule_1", false)], &[]),
            &candle(2, 8.0),
            Mode::Act,
        );
        assert_eq!(exit.orders[0].reason, OrderReason::StopLoss);
        assert_eq!(exit.orders[0].qty, 9);
    }

    #[test]
    fn scripted_port_delivers_events_in_injected_order() {
        let mut port = ScriptedPort::default();
        let req = OrderRequest {
            client_order_id: "x".into(),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 2,
            reason: OrderReason::Rule("r".into()),
            ref_price: 3.0,
            decided_at: 1,
        };
        port.submit(&req).unwrap();
        port.push(OrderEvent::Acked { id: "x".into() });
        port.push(OrderEvent::Filled {
            id: "x".into(),
            qty: 2,
            price: 3.0,
            fees: 0.0,
        });
        assert_eq!(port.submitted, vec![req]);
        assert!(matches!(
            port.drain_events().as_slice(),
            [OrderEvent::Acked { .. }, OrderEvent::Filled { .. }]
        ));
    }

    #[test]
    fn positions_limit_reserves_symbols_and_exit_rules_use_remaining_net() {
        let mut t = trader(
            "TRADE_IN ABC\nRISK MAX_POSITIONS 1\nRISK MAX_ORDERS 1\nWHEN close > 1\nBUY 10\nWHEN close < 1\nSELL 3\nWHEN close < 1\nSELL 5\n",
            1000.0,
        );
        let buy = t
            .on_frame(
                &frame(
                    1,
                    &[("rule_1", true)],
                    &[("rule_2", false), ("rule_3", false)],
                ),
                &candle(1, 10.0),
                Mode::Act,
            )
            .orders
            .remove(0);
        let reset_entry = t.on_frame(
            &frame(
                2,
                &[("rule_1", false)],
                &[("rule_2", false), ("rule_3", false)],
            ),
            &candle(2, 10.0),
            Mode::Act,
        );
        assert!(reset_entry.orders.is_empty());
        fill_buy(&mut t, &buy.client_order_id, 10, 10.0);
        let exits = t.on_frame(
            &frame(
                3,
                &[("rule_1", true)],
                &[("rule_2", true), ("rule_3", true)],
            ),
            &candle(3, 9.0),
            Mode::Act,
        );
        assert_eq!(exits.orders.len(), 2);
        assert!(matches!(
            exits.events[0],
            TraderEvent::RiskBlocked {
                limit: "MAX_POSITIONS",
                ..
            }
        ));
        assert_eq!(
            exits.orders.iter().map(|o| o.qty).collect::<Vec<_>>(),
            vec![3, 5]
        );
        assert_eq!(t.net_position("ABC"), 2);
    }
}
