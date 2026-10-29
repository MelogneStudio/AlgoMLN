//! Portfolio-wide, synchronous order decision and fill ledger.
use crate::{ast::{Action, TradeAmount}, engine::{LevelFrame, Program, RuleId, Symbol}, indicators::Candle};
use std::collections::{BTreeMap, BTreeSet};

pub type OrderId = String;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode { Act, Observe }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side { Buy, Sell }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrderReason { Rule(RuleId), StopLoss, TakeProfit }
#[derive(Clone, Debug, PartialEq)]
pub struct OrderRequest { pub client_order_id: OrderId, pub symbol: Symbol, pub side: Side, pub qty: u64, pub reason: OrderReason, pub ref_price: f64, pub decided_at: i64 }
#[derive(Clone, Debug, PartialEq)]
pub enum OrderEvent { Acked{id:OrderId}, PartialFill{id:OrderId,qty:u64,price:f64,fees:f64}, Filled{id:OrderId,qty:u64,price:f64,fees:f64}, Rejected{id:OrderId,reason:String}, Cancelled{id:OrderId}, Expired{id:OrderId} }
#[derive(Clone, Debug, PartialEq)]
pub enum PortError { Message(String) }
pub trait ExecutionPort { fn submit(&mut self, req:&OrderRequest)->Result<(),PortError>; fn cancel(&mut self,id:OrderId)->Result<(),PortError>; fn drain_events(&mut self)->Vec<OrderEvent>; }
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InstrumentRule { pub lot_size:u64, pub min_qty:u64 }
impl Default for InstrumentRule { fn default()->Self { Self{lot_size:1,min_qty:1} } }
#[derive(Clone, Debug)]
pub struct TraderConfig { pub initial_cash:f64, pub instrument_rules:BTreeMap<Symbol,InstrumentRule> }
impl Default for TraderConfig { fn default()->Self { Self{initial_cash:0.0,instrument_rules:BTreeMap::new()} } }
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TraderError { InitialCashRequired }
#[derive(Clone, Debug, PartialEq)]
pub enum TraderEvent { RuleSkipped{symbol:Symbol,rule_id:RuleId}, RiskBlocked{symbol:Symbol,rule_id:RuleId,limit:&'static str}, OrderEmitted(OrderRequest), Fill{ id:OrderId, qty:u64, price:f64, fees:f64 }, OrderRejected{id:OrderId,reason:String}, OrderClosed{id:OrderId} }
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameOutput { pub orders:Vec<OrderRequest>, pub events:Vec<TraderEvent> }
#[derive(Clone, Debug, Default)]
pub struct Position { pub filled_qty:i64, pub avg_entry:f64 }
#[derive(Clone, Debug)]
struct Pending { req:OrderRequest, filled:u64, acked:bool, terminal:bool }
pub struct Trader { program:Program, cfg:TraderConfig, prev_levels:BTreeMap<(Symbol,RuleId),bool>, positions:BTreeMap<Symbol,Position>, orders:BTreeMap<OrderId,Pending>, cash:f64, realized_pnl_today:f64, accepted_buys_today:u64, next_id:u64 }

impl Trader {
 pub fn new(program:&Program,cfg:TraderConfig)->Result<Self,TraderError>{ if program.risk().max_daily_loss.is_some() && cfg.initial_cash<=0.0{return Err(TraderError::InitialCashRequired)} let cash=cfg.initial_cash; Ok(Self{program:program.clone(),cfg,prev_levels:BTreeMap::new(),positions:BTreeMap::new(),orders:BTreeMap::new(),cash,realized_pnl_today:0.0,accepted_buys_today:0,next_id:1}) }
 pub fn start_day(&mut self,_day:i64){self.realized_pnl_today=0.0;self.accepted_buys_today=0;}
 pub fn cash(&self)->f64{self.cash} pub fn position(&self,symbol:&str)->Option<&Position>{self.positions.get(symbol)}
 pub fn on_order_event(&mut self,ev:OrderEvent){ let id=match &ev{OrderEvent::Acked{id}|OrderEvent::PartialFill{id,..}|OrderEvent::Filled{id,..}|OrderEvent::Rejected{id,..}|OrderEvent::Cancelled{id}|OrderEvent::Expired{id}=>id.clone()}; let mut fill=None; {let Some(o)=self.orders.get_mut(&id) else{return}; match ev {
   OrderEvent::Acked{..}=>o.acked=true,
   OrderEvent::PartialFill{qty,price,fees,..}|OrderEvent::Filled{qty,price,fees,..}=>{ let q=qty.min(o.req.qty.saturating_sub(o.filled)); if q>0 && price.is_finite() && fees.is_finite(){let sym=o.req.symbol.clone();let side=o.req.side;o.filled+=q;fill=Some((sym,side,q,price,fees));} if matches!(ev,OrderEvent::Filled{..}) || o.filled>=o.req.qty{o.terminal=true;} },
   OrderEvent::Rejected{..}|OrderEvent::Cancelled{..}|OrderEvent::Expired{..}=>o.terminal=true,
 }} if let Some((s,side,q,p,fees))=fill{self.apply_fill(&s,side,q,p,fees);} }
 fn apply_fill(&mut self,s:&str,side:Side,q:u64,p:f64,fees:f64){let signed=if side==Side::Buy{q as i64}else{-(q as i64)};let pos=self.positions.entry(s.to_owned()).or_default(); let old=pos.filled_qty; let next=old+signed; if side==Side::Buy{self.cash-=q as f64*p+fees;pos.avg_entry=if next>0{(old.max(0) as f64*pos.avg_entry+q as f64*p)/next as f64}else{p};}else{let close=(q as i64).min(old.max(0)); self.realized_pnl_today+=(p-pos.avg_entry)*close as f64-fees;self.cash+=q as f64*p-fees;if next<=0{pos.avg_entry=0.0;}} pos.filled_qty=next; }
 pub fn on_frame(&mut self,f:&LevelFrame,c:&Candle,mode:Mode)->FrameOutput{let mut out=FrameOutput::default();let mut fires=Vec::new(); for (rid,level) in f.entry.iter().chain(f.exit.iter()){let key=(f.symbol.clone(),rid.clone());let prev=self.prev_levels.insert(key,*level).unwrap_or(false);if *level&&!prev{fires.push((rid.clone(),f.entry.contains_key(rid)));}} if mode==Mode::Observe{return out;}
 let mut full_exit_emitted=false;
 for (rid,is_entry) in fires {let Some(rule)=self.program.rules().iter().find(|r|r.id==rid) else{continue};let action=&rule.action;let side=if is_entry{Side::Buy}else{Side::Sell};let net=self.net_position(&f.symbol);let qty=match action{Action::Buy(a)|Action::Sell(a)=>self.size(a,side,c.close,&f.symbol).unwrap_or(0),Action::SellAll=>net.max(0) as u64};let qty=if is_entry{qty}else{qty.min(net.max(0) as u64)}; if qty==0{out.events.push(TraderEvent::RuleSkipped{symbol:f.symbol.clone(),rule_id:rid});continue;} if is_entry {if self.risk_blocked(&f.symbol,qty){let lim=self.risk_reason(&f.symbol);out.events.push(TraderEvent::RiskBlocked{symbol:f.symbol.clone(),rule_id:rule.id.clone(),limit:lim});continue;}} if !is_entry&&qty as i64>=net{full_exit_emitted=true;} self.emit(&mut out,&f.symbol,side,qty,OrderReason::Rule(rule.id.clone()),c); }
 if !full_exit_emitted {let pos=self.positions.get(&f.symbol).map_or(0,|p|p.filled_qty);let has_exit=self.orders.values().any(|o|!o.terminal&&o.req.symbol==f.symbol&&o.req.side==Side::Sell); if pos>0&&!has_exit {let entry=self.positions[&f.symbol].avg_entry;let sl=self.program.stop_loss().and_then(|x|x.spelling.parse::<f64>().ok()).is_some_and(|pct|c.close<=entry*(1.0-pct/100.0));let tp=self.program.take_profit().and_then(|x|x.spelling.parse::<f64>().ok()).is_some_and(|pct|c.close>=entry*(1.0+pct/100.0));if sl||tp{let reason=if sl{OrderReason::StopLoss}else{OrderReason::TakeProfit};self.emit(&mut out,&f.symbol,Side::Sell,pos as u64,reason,c);}}} out }
 fn net_position(&self,s:&str)->i64{let filled=self.positions.get(s).map_or(0,|p|p.filled_qty);self.orders.values().filter(|o|!o.terminal&&o.req.symbol==s&&o.req.side==Side::Sell).map(|o|(o.req.qty-o.filled) as i64).fold(filled,|a,b|a-b).max(0)}
 fn size(&self,a:&TradeAmount,side:Side,price:f64,s:&str)->Option<u64>{if !price.is_finite()||price<=0.0{return None}let raw=match a{TradeAmount::Quantity(n)=>*n,TradeAmount::Worth(d)=>((d.spelling.parse::<f64>().ok()?)/price).floor() as u64,TradeAmount::BuyCapitalPercentage(p)=>((self.available_cash()*p.spelling.parse::<f64>().ok()?/100.0)/price).floor() as u64,TradeAmount::SellPositionPercentage(p)=>((self.net_position(s) as f64*p.spelling.parse::<f64>().ok()?/100.0).floor()) as u64}; let r=self.cfg.instrument_rules.get(s).copied().unwrap_or_default();let lot=r.lot_size.max(1);let qty=raw/lot*lot;if qty<r.min_qty{None}else{Some(qty)} }
 fn available_cash(&self)->f64{self.cash-self.orders.values().filter(|o|!o.terminal&&o.req.side==Side::Buy).map(|o|(o.req.qty-o.filled) as f64*o.req.ref_price).sum::<f64>()}
 fn risk_reason(&self,s:&str)->&'static str{if self.program.risk().max_positions.is_some_and(|m|{let mut n=self.positions.iter().filter(|(_,p)|p.filled_qty>0).map(|(s,_)|s.clone()).collect::<BTreeSet<_>>();self.orders.values().filter(|o|!o.terminal&&o.req.side==Side::Buy).for_each(|o|{n.insert(o.req.symbol.clone());});n.insert(s.to_owned());n.len() as u64>m}){"MAX_POSITIONS"}else if self.program.risk().max_orders.is_some_and(|m|self.accepted_buys_today+self.orders.values().filter(|o|!o.terminal&&o.req.side==Side::Buy&&!o.acked).count() as u64>=m){"MAX_ORDERS"}else{"MAX_DAILY_LOSS"}}
 fn risk_blocked(&self,s:&str,_q:u64)->bool{let mut symbols=self.positions.iter().filter(|(_,p)|p.filled_qty>0).map(|(s,_)|s.clone()).collect::<BTreeSet<_>>();self.orders.values().filter(|o|!o.terminal&&o.req.side==Side::Buy).for_each(|o|{symbols.insert(o.req.symbol.clone());});symbols.insert(s.to_owned());if self.program.risk().max_positions.is_some_and(|m|symbols.len() as u64>m){return true}if self.program.risk().max_orders.is_some_and(|m|self.accepted_buys_today+self.orders.values().filter(|o|!o.terminal&&o.req.side==Side::Buy&&!o.acked).count() as u64>=m){return true}if self.program.risk().max_daily_loss.as_ref().is_some_and(|m|self.cfg.initial_cash>0.0&&(-self.realized_pnl_today).max(0.0)/self.cfg.initial_cash>=m.spelling.parse::<f64>().unwrap_or(0.0)/100.0){return true} false }
 fn emit(&mut self,out:&mut FrameOutput,s:&str,side:Side,q:u64,reason:OrderReason,c:&Candle){let req=OrderRequest{client_order_id:format!("trader-{}",self.next_id),symbol:s.to_owned(),side,qty:q,reason,ref_price:c.close,decided_at:c.timestamp};self.next_id+=1;if side==Side::Buy{self.accepted_buys_today+=1;}self.orders.insert(req.client_order_id.clone(),Pending{req:req.clone(),filled:0,acked:false,terminal:false});out.events.push(TraderEvent::OrderEmitted(req.clone()));out.orders.push(req);}
}
