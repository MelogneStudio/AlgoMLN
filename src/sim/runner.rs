use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap};

use crate::{
    engine::{Engine, LevelFrame, Program, Symbol},
    indicators::Candle,
    trader::{
        ExecutionPort, Mode, OrderEvent, OrderId, OrderRequest, PortError, Side, Trader,
        TraderConfig, TraderEvent,
    },
};
use super::{
    broker::SimBroker,
    source::{ist_day, limit_to_warmup_window, CandleSource},
    types::{
        round_half_even_i128, EquityPoint, Fill, OpenPosition, RunConfig, RunResult, SimError,
        StageTimings, SymbolEquityPoint, Trade,
    },
};

fn check_invariants(
    trader: &Trader,
    symbols: &[Symbol],
    fills: &[Fill],
    ts: i64,
) -> Result<(), SimError> {
    if trader.cash_paise() < 0 {
        return Err(SimError::Other(format!(
            "invariant: cash is negative at {ts}"
        )));
    }
    if trader.pending_order_count() == 0 && trader.reserved_cash_paise() != 0 {
        return Err(SimError::Other(format!(
            "invariant: reserved cash remains without pending orders at {ts}"
        )));
    }
    for symbol in symbols {
        let signed: i128 = fills
            .iter()
            .filter(|f| &f.symbol == symbol)
            .map(|f| {
                if f.side == Side::Buy {
                    f.qty as i128
                } else {
                    -(f.qty as i128)
                }
            })
            .sum();
        let actual = trader.position(symbol).map_or(0, |p| p.filled_qty) as i128;
        if signed != actual || actual < 0 {
            return Err(SimError::Other(format!(
                "invariant: fill ledger position mismatch for {symbol} at {ts}"
            )));
        }
    }
    Ok(())
}

/// Running per-symbol P&L state, folded forward one fill at a time by the sim
/// frame loop. `cash_flow_paise` is the net cash effect of this symbol's fills
/// (buys negative incl. fees, sells positive net of prorated fees) and `qty` is
/// the open quantity; a symbol's P&L at any mark is `cash_flow_paise + mark*qty`.
/// Replaces the former `symbol_pnl` replay, which re-scanned all fills per symbol
/// per frame (quadratic over a long run).
#[derive(Clone, Copy, Debug, Default)]
struct SymbolPnlAccum {
    qty: u64,
    cash_flow_paise: i128,
}

/// Stream a merged source through independent per-symbol engines, then process
/// broker events before each close frame in deterministic symbol order.
pub fn run(
    program: &Program,
    source: &dyn CandleSource,
    config: RunConfig,
    requested: Option<Vec<Symbol>>,
) -> Result<RunResult, SimError> {
    run_internal(program, source, config, requested, None)
}

/// Precompute each symbol's frames in parallel, then replay through the same
/// trader/broker timeline. This is the Stage-5 path and shares the streaming
/// accounting and event ordering implementation.
pub fn run_precomputed(
    program: &Program,
    source: &dyn CandleSource,
    config: RunConfig,
    requested: Option<Vec<Symbol>>,
) -> Result<RunResult, SimError> {
    let mut symbols = requested
        .clone()
        .unwrap_or_else(|| program.symbols().into_iter().map(str::to_owned).collect());
    symbols.sort();
    symbols.dedup();
    let stream = source.open(
        &symbols,
        config.timeframe,
        config.range,
    )?;
    let stream = limit_to_warmup_window(stream, config.range.from, config.warmup_bars);
    let mut series = BTreeMap::<Symbol, Vec<Candle>>::new();
    for item in stream {
        let (s, c) = item?;
        series.entry(s).or_default().push(c)
    }
    let computed: Vec<Result<(Symbol, Vec<LevelFrame>), crate::engine::EngineError>> = symbols
        .par_iter()
        .map(|s| {
            let candles = series.get(s).map(Vec::as_slice).unwrap_or(&[]);
            crate::engine::evaluate_series(program, s.clone(), candles)
                .map(|frames| (s.clone(), frames))
        })
        .collect();
    let mut frames = HashMap::<(Symbol, i64), LevelFrame>::new();
    for item in computed {
        let (s, values) = item?;
        for frame in values {
            frames.insert((s.clone(), frame.candle_ts), frame);
        }
    }
    run_internal(program, source, config, Some(symbols), Some(frames))
}

fn run_internal(
    program: &Program,
    source: &dyn CandleSource,
    config: RunConfig,
    requested: Option<Vec<Symbol>>,
    mut precomputed: Option<HashMap<(Symbol, i64), LevelFrame>>,
) -> Result<RunResult, SimError> {
    if !config.initial_cash.is_finite() || config.initial_cash <= 0.0 {
        return Err(SimError::Other(
            "initial cash must be finite and positive".into(),
        ));
    }
    if config.latency == 0 {
        return Err(SimError::Other(
            "latency must be at least one candle".into(),
        ));
    }
    if !config.slippage_bps.is_finite() || !(0.0..10000.0).contains(&config.slippage_bps) {
        return Err(SimError::Other(
            "slippage basis points must be in [0,10000)".into(),
        ));
    }
    if [
        config.fee_model.bps,
        config.fee_model.flat_per_order,
        config.fee_model.min_per_order,
    ]
    .iter()
    .any(|x| !x.is_finite() || *x < 0.0)
    {
        return Err(SimError::Other(
            "fee model values must be finite and non-negative".into(),
        ));
    }
    if config
        .partial_fills_pct
        .is_some_and(|p| !p.is_finite() || p <= 0.0 || p > 100.0)
    {
        return Err(SimError::Other(
            "partial fill percentage must be in (0,100]".into(),
        ));
    }
    if config
        .range
        .from
        .zip(config.range.to)
        .is_some_and(|(from, to)| from > to)
    {
        return Err(SimError::Other("--from must be at or before --to".into()));
    }
    let mut symbols =
        requested.unwrap_or_else(|| program.symbols().into_iter().map(str::to_owned).collect());
    symbols.sort();
    symbols.dedup();
    if symbols.is_empty() {
        return Err(SimError::Other("at least one symbol is required".into()));
    }
    let fp = source.fingerprint(&symbols, config.timeframe, config.range);
    if std::env::var_os("ALGOMLN_BENCHMARK").is_none() {
        eprintln!("source fingerprint: {}", fp.0);
    }
    let mut load_time = std::time::Duration::ZERO;
    let mut engine_time = std::time::Duration::ZERO;
    let mut trader_time = std::time::Duration::ZERO;

    let t_load_start = std::time::Instant::now();
    let stream = source.open(
        &symbols,
        config.timeframe,
        config.range,
    )?;
    let mut stream = limit_to_warmup_window(stream, config.range.from, config.warmup_bars);
    load_time += t_load_start.elapsed();

    let t_trader_init = std::time::Instant::now();
    let mut engines: BTreeMap<Symbol, Engine> = symbols
        .iter()
        .map(|s| Ok((s.clone(), Engine::new(program, s.clone())?)))
        .collect::<Result<_, crate::engine::EngineError>>()?;
    let mut trader = Trader::new(
        program,
        TraderConfig {
            initial_cash: config.initial_cash,
            instrument_rules: source.instrument_rules(&symbols),
        },
    )
    .map_err(|e| SimError::Other(format!("trader init: {e:?}")))?;
    let mut broker = SimBroker::new(symbols.clone());
    broker.latency = config.latency;
    broker.slippage_bps = config.slippage_bps;
    broker.tick_sizes_paise = source.tick_sizes_paise(&symbols);
    broker.fees = config.fee_model.clone();
    broker.partial_fills_pct = config.partial_fills_pct;
    let mut fills = Vec::new();
    let mut equity = Vec::new();
    let mut symbol_equity: BTreeMap<Symbol, Vec<SymbolEquityPoint>> =
        symbols.iter().map(|s| (s.clone(), Vec::new())).collect();
    let mut events = Vec::new();
    let mut last = BTreeMap::<Symbol, Candle>::new();
    let mut reqs = HashMap::<OrderId, OrderRequest>::new();
    let mut day = None;
    let mut current_ts = None;
    let mut candles = BTreeMap::<Symbol, Candle>::new();
    let mut observed = BTreeMap::<Symbol, usize>::new();
    // Incremental per-symbol P&L, folded forward as fills arrive instead of
    // replaying the whole fills vector per symbol per frame (was O(frames x fills)).
    let mut symbol_pnl_state = BTreeMap::<Symbol, SymbolPnlAccum>::new();
    let mut processed_fills = 0usize;
    trader_time += t_trader_init.elapsed();
    let mut process = |ts: i64,
                       candles: &BTreeMap<Symbol, Candle>,
                       trader: &mut Trader,
                       broker: &mut SimBroker,
                       engines: &mut BTreeMap<Symbol, Engine>,
                       last: &mut BTreeMap<Symbol, Candle>,
                       fills: &mut Vec<Fill>,
                       events: &mut Vec<TraderEvent>,
                       reqs: &mut HashMap<OrderId, OrderRequest>,
                       equity: &mut Vec<EquityPoint>,
                       symbol_equity: &mut BTreeMap<Symbol, Vec<SymbolEquityPoint>>,
                       day: &mut Option<i64>,
                       observed: &mut BTreeMap<Symbol, usize>,
                       symbol_pnl_state: &mut BTreeMap<Symbol, SymbolPnlAccum>,
                       processed_fills: &mut usize,
                       engine_time: &mut std::time::Duration,
                       trader_time: &mut std::time::Duration|
     -> Result<(), SimError> {
        let t_trader_frame = std::time::Instant::now();
        let d = ist_day(ts);
        if *day != Some(d) {
            trader.start_day(d);
            *day = Some(d);
            if (config.debug_invariants || cfg!(test))
                && (trader.realized_pnl_today_paise() != 0
                    || trader.realized_loss_today_paise() != 0
                    || trader.accepted_buys_today() != 0)
            {
                return Err(SimError::Other(format!(
                    "invariant: daily counters did not reset for IST day {d}"
                )));
            }
        }
        broker.on_candle_open(candles);
        let order_events = broker.drain_events();
        for ev in order_events {
            if let Some(id) = match &ev {
                OrderEvent::PartialFill { id, .. } | OrderEvent::Filled { id, .. } => Some(id),
                _ => None,
            } {
                if let Some(req) = reqs.get(id) {
                    if ts <= req.decided_at && (config.debug_invariants || cfg!(test)) {
                        return Err(SimError::Other(format!(
                            "invariant: order {id} filled before its next candle open"
                        )));
                    }
                }
            }
            if let Some(fill) = to_fill(ts, &ev, reqs) {
                fills.push(fill)
            }
            events.extend(trader.on_order_event(ev));
        }
        *trader_time += t_trader_frame.elapsed();

        let t_eng = std::time::Instant::now();
        let mut frames_vec = Vec::with_capacity(candles.len());
        for (s, c) in candles {
            last.insert(s.clone(), *c);
            let frame = if let Some(frames) = precomputed.as_mut() {
                frames.remove(&(s.clone(), c.timestamp)).ok_or_else(|| {
                    SimError::Other(format!(
                        "precomputed frame missing for {s} at {}",
                        c.timestamp
                    ))
                })?
            } else {
                engines
                    .get_mut(s)
                    .expect("configured symbol")
                    .on_candle(c)?
            };
            frames_vec.push((s, c, frame));
        }
        *engine_time += t_eng.elapsed();

        let t_trader_tail = std::time::Instant::now();
        for (s, c, frame) in frames_vec {
            let count = observed.entry(s.clone()).or_default();
            let mode = if let Some(from) = config.range.from {
                if ts < from {
                    Mode::Observe
                } else {
                    Mode::Act
                }
            } else if *count < config.warmup_bars {
                *count += 1;
                Mode::Observe
            } else {
                Mode::Act
            };
            let out = trader.on_frame(&frame, c, mode);
            events.extend(out.events.clone());
            for id in out.cancels {
                match broker.cancel(id) {
                    Ok(()) => {}
                    Err(PortError::UnknownOrder(_)) => {}
                    Err(e) => return Err(SimError::Other(format!("broker cancel: {e:?}"))),
                }
            }
            for req in &out.orders {
                broker
                    .submit(req)
                    .map_err(|e| SimError::Other(format!("broker submit: {e:?}")))?;
                if reqs
                    .insert(req.client_order_id.clone(), req.clone())
                    .is_some()
                {
                    return Err(SimError::Other(format!(
                        "invariant: duplicate order id {}",
                        req.client_order_id
                    )));
                }
            }
        }
        let mut pos_value_paise = 0i128;
        for s in &symbols {
            if let (Some(p), Some(c)) = (trader.position(s), last.get(s)) {
                let paise = crate::trader::rupees_to_paise(c.close);
                pos_value_paise += p.filled_qty as i128 * paise as i128;
            }
        }
        let cash_paise = trader.cash_paise();
        let positions_value_paise =
            pos_value_paise.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        let equity_paise = cash_paise.saturating_add(positions_value_paise);
        equity.push(EquityPoint {
            ts,
            cash: crate::trader::paise_to_rupees(cash_paise),
            positions_value: crate::trader::paise_to_rupees(positions_value_paise),
            equity: crate::trader::paise_to_rupees(equity_paise),
            cash_paise,
            positions_value_paise,
            equity_paise,
        });
        // Fold the fills added since the last frame into the per-symbol P&L
        // accumulators once, then read each symbol's running total below. This
        // replaces the previous full replay of `fills` per symbol per frame.
        for f in &fills[*processed_fills..] {
            let acc = symbol_pnl_state.entry(f.symbol.clone()).or_default();
            match f.side {
                Side::Buy => {
                    acc.qty += f.qty;
                    acc.cash_flow_paise -=
                        f.price_paise as i128 * f.qty as i128 + f.fees_paise as i128;
                }
                Side::Sell => {
                    let q = f.qty.min(acc.qty);
                    if q > 0 {
                        acc.qty -= q;
                        acc.cash_flow_paise += f.price_paise as i128 * q as i128
                            - round_half_even_i128(
                                f.fees_paise as i128 * q as i128,
                                f.qty.max(1) as i128,
                            );
                    }
                }
            }
        }
        *processed_fills = fills.len();
        let initial_cash_paise = crate::trader::rupees_to_paise(config.initial_cash);
        let allocation_base = initial_cash_paise / symbols.len().max(1) as i64;
        let allocation_remainder = initial_cash_paise.rem_euclid(symbols.len().max(1) as i64);
        for (symbol_index, symbol) in symbols.iter().enumerate() {
            let mark = last
                .get(symbol)
                .map_or(0, |c| crate::trader::rupees_to_paise(c.close));
            // Running P&L against the current mark: identical to the old
            // `symbol_pnl` replay, but O(1) per symbol per frame.
            let (pnl, qty) = symbol_pnl_state.get(symbol).map_or((0i64, 0i64), |acc| {
                let pnl_paise = acc.cash_flow_paise + mark as i128 * acc.qty as i128;
                (
                    pnl_paise.clamp(i64::MIN as i128, i64::MAX as i128) as i64,
                    acc.qty as i64,
                )
            });
            // `allocation_base` is a static even split of initial cash used only as
            // the per-symbol baseline; the series tracks relative P&L and does not
            // attribute pooled or reserved cash per symbol, so it need not reconcile
            // exactly to portfolio equity while cash is reserved for pending orders.
            let allocation_paise =
                allocation_base + i64::from((symbol_index as i64) < allocation_remainder);
            let symbol_equity_paise = allocation_paise.saturating_add(pnl);
            symbol_equity
                .entry(symbol.clone())
                .or_default()
                .push(SymbolEquityPoint {
                    ts,
                    equity: crate::trader::paise_to_rupees(symbol_equity_paise),
                    equity_paise: symbol_equity_paise,
                    position_qty: qty,
                });
        }
        if config.debug_invariants || cfg!(test) {
            check_invariants(trader, &symbols, fills, ts)?;
        }
        *trader_time += t_trader_tail.elapsed();
        Ok(())
    };
    const LOAD_SAMPLE_INTERVAL: u64 = 64;
    let mut load_sample_time = std::time::Duration::ZERO;
    let mut load_sample_count = 0u64;
    let mut total_loaded_items = 0u64;
    loop {
        let next_item = if total_loaded_items % LOAD_SAMPLE_INTERVAL == 0 {
            let t_load = std::time::Instant::now();
            let item = stream.next();
            load_sample_time += t_load.elapsed();
            load_sample_count += 1;
            item
        } else {
            stream.next()
        };
        total_loaded_items += 1;
        let Some(item) = next_item else {
            break;
        };
        let (s, c) = item?;
        if current_ts.is_some_and(|t| t != c.timestamp) {
            process(
                current_ts.unwrap(),
                &candles,
                &mut trader,
                &mut broker,
                &mut engines,
                &mut last,
                &mut fills,
                &mut events,
                &mut reqs,
                &mut equity,
                &mut symbol_equity,
                &mut day,
                &mut observed,
                &mut symbol_pnl_state,
                &mut processed_fills,
                &mut engine_time,
                &mut trader_time,
            )?;
            candles.clear();
        }
        current_ts = Some(c.timestamp);
        if candles.contains_key(&s) {
            return Err(SimError::Other(format!(
                "duplicate candle for symbol {s:?} at timestamp {}",
                c.timestamp
            )));
        }
        candles.insert(s, c);
    }
    if load_sample_count > 0 {
        load_time += load_sample_time.mul_f64(total_loaded_items as f64 / load_sample_count as f64);
    }
    if let Some(ts) = current_ts {
        process(
            ts,
            &candles,
            &mut trader,
            &mut broker,
            &mut engines,
            &mut last,
            &mut fills,
            &mut events,
            &mut reqs,
            &mut equity,
            &mut symbol_equity,
            &mut day,
            &mut observed,
            &mut symbol_pnl_state,
            &mut processed_fills,
            &mut engine_time,
            &mut trader_time,
        )?;
    }
    let t_trader_end = std::time::Instant::now();
    if config.debug_invariants || cfg!(test) {
        if let Some(last_point) = equity.last() {
            let mut qty = BTreeMap::<Symbol, u64>::new();
            let mut cash_paise = crate::trader::rupees_to_paise(config.initial_cash) as i128;
            for fill in &fills {
                let q = qty.entry(fill.symbol.clone()).or_default();
                match fill.side {
                    Side::Buy => {
                        *q += fill.qty;
                        cash_paise -=
                            fill.price_paise as i128 * fill.qty as i128 + fill.fees_paise as i128;
                    }
                    Side::Sell => {
                        if fill.qty > *q {
                            return Err(SimError::Other(format!(
                                "invariant: sell fill exceeds position for {}",
                                fill.symbol
                            )));
                        }
                        *q -= fill.qty;
                        cash_paise +=
                            fill.price_paise as i128 * fill.qty as i128 - fill.fees_paise as i128;
                    }
                }
            }
            let positions_value_paise: i128 = symbols
                .iter()
                .map(|s| {
                    let q = *qty.get(s).unwrap_or(&0);
                    let mark = last
                        .get(s)
                        .map_or(0, |x| crate::trader::rupees_to_paise(x.close));
                    mark as i128 * q as i128
                })
                .sum();
            let recomputed_equity = cash_paise + positions_value_paise;
            if cash_paise != last_point.cash_paise as i128
                || positions_value_paise != last_point.positions_value_paise as i128
                || recomputed_equity != last_point.equity_paise as i128
            {
                return Err(SimError::Other(
                    "invariant: paise cash and positions recomputed from fills do not exactly reconcile to final equity".into(),
                ));
            }
        }
    }
    let open_positions = symbols
        .iter()
        .filter_map(|s| {
            let p = trader.position(s)?;
            if p.filled_qty == 0 {
                return None;
            }
            let c = last.get(s)?;
            let last_close_paise = crate::trader::rupees_to_paise(c.close);
            let unrealized_paise =
                (last_close_paise - p.avg_entry_paise).saturating_mul(p.filled_qty);
            Some(OpenPosition {
                symbol: s.clone(),
                qty: p.filled_qty,
                avg_entry: crate::trader::paise_to_rupees(p.avg_entry_paise),
                last_close: crate::trader::paise_to_rupees(last_close_paise),
                unrealized: crate::trader::paise_to_rupees(unrealized_paise),
                avg_entry_paise: p.avg_entry_paise,
                last_close_paise,
                unrealized_paise,
            })
        })
        .collect();
    let mut ph = 0xcbf29ce484222325u64;
    for b in format!("{:?}", program.strategy()).bytes() {
        ph ^= b as u64;
        ph = ph.wrapping_mul(0x100000001b3);
    }
    let source_stats = source.stats();
    // Baked at build time by build.rs (reproducibility only); never spawns a
    // process per run. Absent in non-git/sandboxed builds, recorded as null.
    let commit = option_env!("GIT_COMMIT").map(str::to_owned);
    let manifest = serde_json::json!({
        "program_hash": format!("fnv1a64:{ph:016x}"),
        "source_fingerprint": fp,
        "symbols": symbols,
        "timeframe": config.timeframe,
        "range": config.range,
        "config": config,
        "tool_version": env!("CARGO_PKG_VERSION"),
        "git_commit": commit,
        "source_stats": source_stats
    });
    if std::env::var_os("ALGOMLN_BENCHMARK").is_none() {
        eprintln!("source fingerprint: {}", fp.0);
    }
    let trades = derive_trades(&fills);
    trader_time += t_trader_end.elapsed();
    Ok(RunResult {
        fills,
        equity,
        events,
        open_positions,
        manifest,
        source_stats,
        trades,
        symbol_equity,
        timings: StageTimings {
            load_merge: load_time,
            engine_indicators: engine_time,
            trader: trader_time,
        },
    })
}

fn to_fill(ts: i64, e: &OrderEvent, reqs: &HashMap<OrderId, OrderRequest>) -> Option<Fill> {
    let (id, qty, price, fees) = match e {
        OrderEvent::Filled {
            id,
            qty,
            price,
            fees,
        }
        | OrderEvent::PartialFill {
            id,
            qty,
            price,
            fees,
        } => (id, *qty, *price, *fees),
        _ => return None,
    };
    let r = reqs.get(id)?;
    let price = crate::trader::paise_to_rupees(crate::trader::rupees_to_paise(price));
    let fees = crate::trader::paise_to_rupees(crate::trader::rupees_to_paise(fees));
    Some(Fill {
        ts,
        symbol: r.symbol.clone(),
        order_id: id.clone(),
        side: r.side,
        qty,
        price,
        price_paise: crate::trader::rupees_to_paise(price),
        fees,
        fees_paise: crate::trader::rupees_to_paise(fees),
        reason: r.reason.clone(),
    })
}

fn derive_trades(fills: &[Fill]) -> Vec<Trade> {
    #[derive(Default)]
    struct Lot {
        qty: u64,
        cost_paise: i128,
        entry_fees_paise: i128,
        entry_ts: i64,
        closed_qty: u64,
        closed_entry_cost_paise: i128,
        closed_exit_value_paise: i128,
        closed_fees_paise: i128,
        closed_pnl_paise: i128,
    }
    let mut lots = BTreeMap::<Symbol, Lot>::new();
    let mut trades = Vec::new();
    for f in fills {
        let lot = lots.entry(f.symbol.clone()).or_default();
        match f.side {
            Side::Buy => {
                if lot.qty == 0 {
                    lot.entry_ts = f.ts;
                }
                lot.qty += f.qty;
                lot.cost_paise += f.price_paise as i128 * f.qty as i128;
                lot.entry_fees_paise += f.fees_paise as i128;
            }
            Side::Sell => {
                if lot.qty == 0 {
                    continue;
                }
                let q = f.qty.min(lot.qty);
                let entry_cost = round_half_even_i128(lot.cost_paise * q as i128, lot.qty as i128);
                let entry_fee =
                    round_half_even_i128(lot.entry_fees_paise * q as i128, lot.qty as i128);
                let exit_fee =
                    round_half_even_i128(f.fees_paise as i128 * q as i128, f.qty.max(1) as i128);
                let exit_value = f.price_paise as i128 * q as i128;
                lot.closed_qty += q;
                lot.closed_entry_cost_paise += entry_cost;
                lot.closed_exit_value_paise += exit_value;
                lot.closed_fees_paise += entry_fee + exit_fee;
                lot.closed_pnl_paise += exit_value - entry_cost - entry_fee - exit_fee;
                lot.qty -= q;
                lot.cost_paise -= entry_cost;
                lot.entry_fees_paise -= entry_fee;
                if lot.qty == 0 && lot.closed_qty > 0 {
                    trades.push(Trade {
                        symbol: f.symbol.clone(),
                        entry_ts: lot.entry_ts,
                        exit_ts: f.ts,
                        qty: lot.closed_qty,
                        avg_entry: crate::trader::paise_to_rupees(round_half_even_i128(
                            lot.closed_entry_cost_paise,
                            lot.closed_qty as i128,
                        ) as i64),
                        avg_entry_paise: round_half_even_i128(
                            lot.closed_entry_cost_paise,
                            lot.closed_qty as i128,
                        ) as i64,
                        avg_exit: crate::trader::paise_to_rupees(round_half_even_i128(
                            lot.closed_exit_value_paise,
                            lot.closed_qty as i128,
                        ) as i64),
                        avg_exit_paise: round_half_even_i128(
                            lot.closed_exit_value_paise,
                            lot.closed_qty as i128,
                        ) as i64,
                        fees: crate::trader::paise_to_rupees(lot.closed_fees_paise as i64),
                        fees_paise: lot.closed_fees_paise as i64,
                        net_pnl: crate::trader::paise_to_rupees(lot.closed_pnl_paise as i64),
                        net_pnl_paise: lot.closed_pnl_paise as i64,
                        exit_reason: f.reason.clone(),
                    });
                    *lot = Lot::default();
                }
            }
        }
    }
    trades.sort_by(|a, b| (a.exit_ts, &a.symbol).cmp(&(b.exit_ts, &b.symbol)));
    trades
}
