use mln_check::Tf;
use mln_exec::{
    align_symbol_bars, Bar, BarOutput, ExecError, Executor, NodeVal, PositionSide, PositionTracker,
    PositionView, SessionCalendar,
};
use mln_ir::{
    lower_default, IrClock, IrConst, IrGraph, IrOp, IrReset, IrStateVar, IrTy, SeriesField, StateId,
};
use mln_lang::{parse, CmpOp, TfUnit};
use mln_std::TriBool;

fn compile(src: &str) -> IrGraph {
    let parsed = parse(src);
    assert!(
        parsed.diags.is_empty(),
        "parse failed: {:?}\nsrc:\n{src}",
        parsed.diags
    );
    lower_default(&parsed.program).unwrap_or_else(|e| panic!("lower failed: {e:?}\nsrc:\n{src}"))
}

fn binding_num(exec: &Executor, ir: &IrGraph, name: &str) -> f64 {
    let id = ir
        .bindings
        .iter()
        .find(|(k, _)| k == name)
        .unwrap_or_else(|| panic!("missing binding `{name}`"))
        .1;
    match exec.node_value(id).expect("node val") {
        NodeVal::Num(v) => v,
        other => panic!("binding `{name}` expected Num, got {other:?}"),
    }
}

fn binding_bool(exec: &Executor, ir: &IrGraph, name: &str) -> TriBool {
    let id = ir
        .bindings
        .iter()
        .find(|(k, _)| k == name)
        .unwrap_or_else(|| panic!("missing binding `{name}`"))
        .1;
    match exec.node_value(id).expect("node val") {
        NodeVal::Bool(b) => b,
        other => panic!("binding `{name}` expected Bool, got {other:?}"),
    }
}

fn build_day_1m_bars(day_start_ts: i64, bars_in_day: usize, base_price: f64) -> Vec<Bar> {
    (0..bars_in_day)
        .map(|m| {
            let ts = day_start_ts + (m as i64) * 60;
            let open = base_price + (m as f64) * 0.05;
            let close = open + 0.03;
            let high = close + 0.1;
            let low = open - 0.1;
            Bar::new(ts, open, high, low, close, 100.0)
        })
        .collect()
}

#[test]
fn t5_01_def_call_sites_independent_state() {
    let src = "\
def momentum_gap(s) = ema(s, 3) - sma(s, 5)
on_close = momentum_gap(close)
on_open = momentum_gap(open)
on_vol = momentum_gap(volume)
ref_close = ema(close, 3) - sma(close, 5)
ref_open = ema(open, 3) - sma(open, 5)
entry = on_close > on_open
exit = on_vol < 0.0
";
    let ir = compile(src);
    let mut exec = Executor::new(&ir).expect("exec");

    for i in 0..20 {
        let ts = 1_700_000_000 + (i as i64) * 60;
        let close = 100.0 + (i as f64) * 1.5;
        let open = 100.0 - (i as f64) * 0.5;
        let volume = 500.0 + ((i % 5) as f64) * 50.0;
        exec.step(Bar::new(
            ts,
            open,
            open.max(close) + 1.0,
            open.min(close) - 1.0,
            close,
            volume,
        ))
        .expect("step");

        let oc = binding_num(&exec, &ir, "on_close");
        let oo = binding_num(&exec, &ir, "on_open");
        let ov = binding_num(&exec, &ir, "on_vol");
        let rc = binding_num(&exec, &ir, "ref_close");
        let ro = binding_num(&exec, &ir, "ref_open");

        if i >= 4 {
            assert!((oc - rc).abs() < 1e-12);
            assert!((oo - ro).abs() < 1e-12);
            assert!((oc - oo).abs() > 1e-3, "bar {i}: call sites collided");
            assert!((oc - ov).abs() > 1e-3);
        } else {
            assert!(oc.is_nan() && oo.is_nan() && ov.is_nan());
        }
    }
}

#[test]
fn t5_02_no_lookahead_daily_close() {
    let src = "\
daily_close = close @ 1d
daily_sma2 = sma(close, 2) @ 1d
entry = close > daily_close
exit = close < daily_sma2
";
    let ir = compile(src);

    // 3 UTC days starting 2023-11-15 09:15 UTC = 1_700_039_700
    let day0 = 1_700_006_400 + 9 * 3600 + 15 * 60;
    let day1 = day0 + 86_400;
    let day2 = day0 + 2 * 86_400;

    let mut all_bars = Vec::new();
    all_bars.extend(build_day_1m_bars(day0, 20, 100.0));
    let expected_day0_close = all_bars.last().unwrap().close;
    all_bars.extend(build_day_1m_bars(day1, 20, 200.0));
    let expected_day1_close = all_bars.last().unwrap().close;
    all_bars.extend(build_day_1m_bars(day2, 20, 300.0));

    let mut exec = Executor::new(&ir).expect("exec");
    let mut daily_vals = Vec::with_capacity(all_bars.len());
    for (idx, &bar) in all_bars.iter().enumerate() {
        exec.step(bar).expect("step");
        let dc = binding_num(&exec, &ir, "daily_close");
        let dsma = binding_num(&exec, &ir, "daily_sma2");
        daily_vals.push(dc);

        if idx < 20 {
            assert!(
                dc.is_nan(),
                "day 0 bar {idx} leaked future daily close {dc}"
            );
            assert!(dsma.is_nan());
        } else if idx < 40 {
            assert_eq!(dc, expected_day0_close, "day 1 bar {idx} mismatch");
            assert!(dsma.is_nan(), "sma(2) @ 1d needs 2 completed days");
        } else {
            assert_eq!(dc, expected_day1_close, "day 2 bar {idx} mismatch");
            let expected_sma = (expected_day0_close + expected_day1_close) / 2.0;
            assert!((dsma - expected_sma).abs() < 1e-12);
        }
    }

    // Truncated run through end of day 1 matches prefix of full run on every single bar.
    let mut truncated = Executor::new(&ir).expect("trunc exec");
    for (idx, &bar) in all_bars[..40].iter().enumerate() {
        truncated.step(bar).expect("trunc step");
        let dc = binding_num(&truncated, &ir, "daily_close");
        if daily_vals[idx].is_nan() {
            assert!(dc.is_nan());
        } else {
            assert_eq!(dc, daily_vals[idx]);
        }
    }
}

#[test]
fn t5_03_session_alignment_360_and_375_bar_days() {
    let src = "\
d_open = open @ 1d
d_high = high @ 1d
d_low = low @ 1d
d_close = close @ 1d
d_vol = sum(volume, 2) @ 1d
entry = close > d_close
exit = false
";
    let ir = compile(src);
    let day0 = 1_700_006_400 + 9 * 3600 + 15 * 60; // 09:15 UTC
    let day1 = day0 + 86_400;
    let day2 = day0 + 2 * 86_400;

    let bars_d0 = build_day_1m_bars(day0, 360, 100.0);
    let bars_d1 = build_day_1m_bars(day1, 375, 150.0);
    let bars_d2 = build_day_1m_bars(day2, 360, 220.0);

    let d0_open = bars_d0[0].open;
    let d0_high = bars_d0
        .iter()
        .map(|b| b.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let d0_low = bars_d0.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    let d0_close = bars_d0.last().unwrap().close;
    let d0_vol: f64 = bars_d0.iter().map(|b| b.volume).sum();

    let d1_open = bars_d1[0].open;
    let d1_high = bars_d1
        .iter()
        .map(|b| b.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let d1_low = bars_d1.iter().map(|b| b.low).fold(f64::INFINITY, f64::min);
    let d1_close = bars_d1.last().unwrap().close;
    let d1_vol: f64 = bars_d1.iter().map(|b| b.volume).sum();

    let mut all = Vec::new();
    all.extend_from_slice(&bars_d0);
    all.extend_from_slice(&bars_d1);
    all.extend_from_slice(&bars_d2);

    let mut exec = Executor::new(&ir).expect("exec");
    for (idx, &bar) in all.iter().enumerate() {
        exec.step(bar).expect("step");
        if idx < 360 {
            assert!(binding_num(&exec, &ir, "d_close").is_nan());
        } else if idx < 360 + 375 {
            assert_eq!(binding_num(&exec, &ir, "d_open"), d0_open);
            assert_eq!(binding_num(&exec, &ir, "d_high"), d0_high);
            assert_eq!(binding_num(&exec, &ir, "d_low"), d0_low);
            assert_eq!(binding_num(&exec, &ir, "d_close"), d0_close);
            assert!(binding_num(&exec, &ir, "d_vol").is_nan());
        } else {
            assert_eq!(binding_num(&exec, &ir, "d_open"), d1_open);
            assert_eq!(binding_num(&exec, &ir, "d_high"), d1_high);
            assert_eq!(binding_num(&exec, &ir, "d_low"), d1_low);
            assert_eq!(binding_num(&exec, &ir, "d_close"), d1_close);
            assert!((binding_num(&exec, &ir, "d_vol") - (d0_vol + d1_vol)).abs() < 1e-6);
        }
    }
}

#[test]
fn t5_04_state_resets_only_on_declared_event() {
    let src = "\
state on_exit_var:
    init = 10.0
    reset = exit
state on_cond_var:
    init = 0.0
    reset = close < 95.0
state never_resets:
    init = 1.0
on true:
    never_resets = never_resets + 1.0
    on_cond_var = on_cond_var + 1.0
while long:
    on_exit_var = on_exit_var + 5.0
entry = on_exit_var > 0.0 and never_resets > 0.0
exit = on_cond_var >= 0.0
";
    let ir = compile(src);
    let mut exec = Executor::new(&ir).expect("exec");

    let flat = PositionView::default();
    let long = PositionView {
        side: PositionSide::Long,
        entry_price: 100.0,
        bars_held: 1,
        pnl: 0.0,
    };

    // Bar 0 (flat): init values + on true runs.
    exec.step_with_context(Bar::new(100, 100.0, 101.0, 99.0, 100.0, 10.0), flat, &[])
        .unwrap();
    assert_eq!(exec.state_value("on_exit_var"), Some(NodeVal::Num(10.0)));
    assert_eq!(exec.state_value("on_cond_var"), Some(NodeVal::Num(1.0)));
    assert_eq!(exec.state_value("never_resets"), Some(NodeVal::Num(2.0)));

    // Bar 1 (enter long): `while long` adds 5.
    exec.step_with_context(Bar::new(160, 100.0, 101.0, 99.0, 100.0, 10.0), long, &[])
        .unwrap();
    assert_eq!(exec.state_value("on_exit_var"), Some(NodeVal::Num(15.0)));
    assert_eq!(exec.state_value("on_cond_var"), Some(NodeVal::Num(2.0)));
    assert_eq!(exec.state_value("never_resets"), Some(NodeVal::Num(3.0)));

    // Bar 2 (still long, close < 95 triggers custom reset on `on_cond_var`, then `on true` adds 1 -> 1.0).
    exec.step_with_context(Bar::new(220, 96.0, 97.0, 94.0, 94.0, 10.0), long, &[])
        .unwrap();
    assert_eq!(exec.state_value("on_exit_var"), Some(NodeVal::Num(20.0)));
    assert_eq!(exec.state_value("on_cond_var"), Some(NodeVal::Num(1.0)));
    assert_eq!(exec.state_value("never_resets"), Some(NodeVal::Num(4.0)));

    // Bar 3 (exit back to flat): ONLY `on_exit_var` resets to 10.0; `never_resets` keeps incrementing!
    exec.step_with_context(Bar::new(280, 95.0, 96.0, 94.0, 95.5, 10.0), flat, &[])
        .unwrap();
    assert_eq!(exec.state_value("on_exit_var"), Some(NodeVal::Num(10.0)));
    assert_eq!(exec.state_value("on_cond_var"), Some(NodeVal::Num(2.0)));
    assert_eq!(exec.state_value("never_resets"), Some(NodeVal::Num(5.0)));
}

#[test]
fn t5_05_while_long_updates_only_while_long() {
    let src = "\
state peak:
    init = 0.0
    reset = exit
state in_long_sum:
    init = 0.0
while long:
    peak = max(peak, high)
    in_long_sum = cumsum(close)
entry = peak > 0.0 and in_long_sum >= 0.0
exit = false
";
    let ir = compile(src);
    let mut exec = Executor::new(&ir).expect("exec");

    let flat = PositionView::default();
    let long = PositionView {
        side: PositionSide::Long,
        entry_price: 100.0,
        bars_held: 1,
        pnl: 0.0,
    };
    let short = PositionView {
        side: PositionSide::Short,
        entry_price: 100.0,
        bars_held: 1,
        pnl: 0.0,
    };

    // Flat bars: state must NOT update (`peak == 0`, `in_long_sum == 0`, cumsum untouched).
    exec.step_with_context(Bar::new(60, 100.0, 150.0, 99.0, 105.0, 10.0), flat, &[])
        .unwrap();
    assert_eq!(exec.state_value("peak"), Some(NodeVal::Num(0.0)));
    assert_eq!(exec.state_value("in_long_sum"), Some(NodeVal::Num(0.0)));

    // Short bars: `while long` guard still inactive.
    exec.step_with_context(Bar::new(120, 105.0, 200.0, 99.0, 110.0, 10.0), short, &[])
        .unwrap();
    assert_eq!(exec.state_value("peak"), Some(NodeVal::Num(0.0)));
    assert_eq!(exec.state_value("in_long_sum"), Some(NodeVal::Num(0.0)));

    // Enter Long: `while long` sets `peak = max(0, 104) = 104`, cumsum = 100.
    exec.step_with_context(Bar::new(180, 99.0, 104.0, 98.0, 100.0, 10.0), long, &[])
        .unwrap();
    assert_eq!(exec.state_value("peak"), Some(NodeVal::Num(104.0)));
    assert_eq!(exec.state_value("in_long_sum"), Some(NodeVal::Num(100.0)));

    // Still Long: `peak = max(104, 112) = 112`, cumsum = 100 + 108 = 208.
    exec.step_with_context(Bar::new(240, 100.0, 112.0, 100.0, 108.0, 10.0), long, &[])
        .unwrap();
    assert_eq!(exec.state_value("peak"), Some(NodeVal::Num(112.0)));
    assert_eq!(exec.state_value("in_long_sum"), Some(NodeVal::Num(208.0)));

    // Exit to Flat: `peak` resets, `in_long_sum` stays frozen at 208 (not updated while flat).
    exec.step_with_context(Bar::new(300, 108.0, 500.0, 107.0, 300.0, 10.0), flat, &[])
        .unwrap();
    assert_eq!(exec.state_value("peak"), Some(NodeVal::Num(0.0)));
    assert_eq!(exec.state_value("in_long_sum"), Some(NodeVal::Num(208.0)));
}

#[test]
fn t5_06_position_bars_held_increments_and_resets_on_exit() {
    let src = "\
held = position.bars_held
entry_px = position.entry_price
cur_pnl = position.pnl
is_long = position.side == \"long\"
entry = position.side == \"flat\" and close > open
exit = is_long and held >= 3
";
    let ir = compile(src);
    let mut exec = Executor::new(&ir).expect("exec");
    let mut tracker = PositionTracker::new();

    let bars = [
        Bar::new(60, 100.0, 103.0, 99.0, 102.0, 10.0), // signal entry at close=102
        Bar::new(120, 102.0, 105.0, 101.0, 104.0, 10.0), // bar 1 in long: bars_held=1, pnl=2
        Bar::new(180, 104.0, 106.0, 103.0, 105.0, 10.0), // bar 2 in long: bars_held=2, pnl=3
        Bar::new(240, 105.0, 108.0, 104.0, 107.0, 10.0), // bar 3 in long: bars_held=3, pnl=5 -> signal exit
        Bar::new(300, 107.0, 107.5, 105.0, 106.0, 10.0), // flat again: bars_held=0
    ];

    let mut held_trace = Vec::new();
    let mut exit_signals = Vec::new();
    for bar in &bars {
        let pos = tracker.begin_bar(bar);
        let out = exec.step_with_context(*bar, pos, &[]).expect("step");
        held_trace.push(binding_num(&exec, &ir, "held") as u64);
        exit_signals.push(out.exit_signal);
        if held_trace.last().copied().unwrap() > 0 {
            assert_eq!(binding_bool(&exec, &ir, "is_long"), TriBool::True);
            assert_eq!(binding_num(&exec, &ir, "entry_px"), 102.0);
            assert!((binding_num(&exec, &ir, "cur_pnl") - (bar.close - 102.0)).abs() < 1e-12);
        } else {
            assert_eq!(binding_bool(&exec, &ir, "is_long"), TriBool::False);
            assert!(binding_num(&exec, &ir, "entry_px").is_nan());
        }
        tracker.end_bar(bar, &out);
    }

    assert_eq!(held_trace, vec![0, 1, 2, 3, 0]);
    assert_eq!(exit_signals, vec![0, 0, 0, 1, 0]);
}

#[test]
fn t5_07_daily_series_skip_weekends_and_holidays() {
    let src = "\
d_sma3 = sma(close, 3) @ 1d
entry = close > d_sma3
exit = false
";
    let ir = compile(src);
    // 2023-11-13 is Monday (day 19674 * 86400 = 1_699_833_600)
    let mon = 1_699_833_600 + 9 * 3600 + 15 * 60;
    let tue = mon + 86_400;
    // Wed (mon + 2d) is a market holiday -> zero bars!
    let thu = mon + 3 * 86_400;
    let fri = mon + 4 * 86_400;
    // Sat, Sun -> zero bars!
    let next_mon = mon + 7 * 86_400;

    let mut bars = Vec::new();
    bars.extend(build_day_1m_bars(mon, 10, 100.0));
    let c_mon = bars.last().unwrap().close;
    bars.extend(build_day_1m_bars(tue, 10, 110.0));
    let c_tue = bars.last().unwrap().close;
    bars.extend(build_day_1m_bars(thu, 10, 120.0));
    let c_thu = bars.last().unwrap().close;
    bars.extend(build_day_1m_bars(fri, 10, 130.0));
    let c_fri = bars.last().unwrap().close;
    bars.extend(build_day_1m_bars(next_mon, 5, 140.0));

    let mut exec = Executor::new(&ir).expect("exec");
    let mut fri_sma = f64::NAN;
    let mut next_mon_sma = f64::NAN;
    for (idx, &bar) in bars.iter().enumerate() {
        exec.step(bar).expect("step");
        let s = binding_num(&exec, &ir, "d_sma3");
        if idx < 30 {
            assert!(s.is_nan(), "idx {idx} expected NaN before 3 trading days");
        } else if idx < 40 {
            fri_sma = s;
        } else {
            next_mon_sma = s;
        }
    }

    // On Friday, the 3 completed trading days are [Mon, Tue, Thu] (Wednesday skipped cleanly).
    assert!((fri_sma - (c_mon + c_tue + c_thu) / 3.0).abs() < 1e-12);
    // Next Monday, the 3 completed trading days are [Tue, Thu, Fri] (weekend skipped cleanly).
    assert!((next_mon_sma - (c_tue + c_thu + c_fri) / 3.0).abs() < 1e-12);
}

#[test]
fn t5_08_multi_symbol_missing_bars_hold_last_and_stale() {
    let src = "\
infy_c = symbol(\"INFY\").close
infy_stale = symbol(\"INFY\").stale
infy_d = symbol(\"INFY\").close @ 1d
infy_d_stale = symbol(\"INFY\").stale @ 1d
entry = not infy_stale and close > infy_c
exit = infy_stale or (infy_d > 0.0 and infy_d_stale)
";
    let ir = compile(src);
    let d0 = 1_700_006_400 + 9 * 3600 + 15 * 60;
    let d1 = d0 + 86_400;
    let d2 = d0 + 2 * 86_400;

    // Primary trades on day 0, day 1, day 2 (2 bars each day).
    let primary = vec![
        Bar::new(d0, 100.0, 101.0, 99.0, 100.5, 10.0),
        Bar::new(d0 + 60, 100.5, 102.0, 100.0, 101.0, 10.0),
        Bar::new(d1, 101.0, 102.0, 100.0, 101.5, 10.0),
        Bar::new(d1 + 60, 101.5, 103.0, 101.0, 102.0, 10.0),
        Bar::new(d2, 102.0, 103.0, 101.0, 102.5, 10.0),
    ];

    // Secondary INFY has: day 0 bar 0 only (bar 1 missing), ENTIRE day 1 missing, day 2 bar 0 present.
    let infy = vec![
        Bar::new(d0, 500.0, 502.0, 499.0, 501.0, 50.0),
        Bar::new(d2, 510.0, 512.0, 509.0, 511.0, 50.0),
    ];

    let aligned = align_symbol_bars(&primary, &infy).expect("align");
    let mut exec = Executor::new(&ir).expect("exec");

    let mut observed = Vec::new();
    for (&pbar, &sbar) in primary.iter().zip(aligned.iter()) {
        exec.step_with_context(pbar, PositionView::default(), &[sbar])
            .expect("step");
        observed.push((
            binding_num(&exec, &ir, "infy_c"),
            binding_bool(&exec, &ir, "infy_stale"),
            binding_num(&exec, &ir, "infy_d"),
            binding_bool(&exec, &ir, "infy_d_stale"),
        ));
    }

    assert_eq!(observed[0].0, 501.0);
    assert_eq!(observed[0].1, TriBool::False);
    // Missing bar 1 holds 501.0 and marks stale = True
    assert_eq!(observed[1].0, 501.0);
    assert_eq!(observed[1].1, TriBool::True);
    // Day 1 missing holds 501.0, daily close sees day 0 completed close = 501.0 (not stale)
    assert_eq!(observed[2].0, 501.0);
    assert_eq!(observed[2].1, TriBool::True);
    assert_eq!(observed[2].2, 501.0);
    assert_eq!(observed[2].3, TriBool::False);
    // Day 2: bar 0 fresh base value = 511.0, while daily completed Day 1 was missing -> holds 501.0 & stale @ 1d = True!
    assert_eq!(observed[4].0, 511.0);
    assert_eq!(observed[4].1, TriBool::False);
    assert_eq!(observed[4].2, 501.0);
    assert_eq!(observed[4].3, TriBool::True);
}

#[test]
fn t5_09_chunked_equals_bar_by_bar_phase5() {
    let src = "\
def diff_ema(x) = ema(x, 4) - ema(x, 8)
state peak:
    init = 0.0
    reset = exit
on enter:
    peak = close
while long:
    peak = max(peak, high)
d_close = close @ 1d
h_sma = sma(close, 3) @ 1h
sym_c = symbol(\"INFY\").close @ 1d
sym_stale = symbol(\"INFY\").stale
sig = diff_ema(close)
entry = position.side == \"flat\" and not sym_stale and d_close > 0.0 and h_sma > 0.0 and sig > -10.0 and close > sym_c * 0.1
exit = position.side == \"long\" and (close < peak * 0.99 or position.bars_held >= 4)
";
    let ir = compile(src);
    let d0 = 1_700_006_400 + 9 * 3600;
    let d1 = d0 + 86_400;
    let d2 = d0 + 2 * 86_400;

    let mut primary = Vec::new();
    primary.extend(build_day_1m_bars(d0, 70, 100.0));
    primary.extend(build_day_1m_bars(d1, 70, 105.0));
    primary.extend(build_day_1m_bars(d2, 30, 110.0));

    // Drop every 7th bar of INFY.
    let infy_raw: Vec<Bar> = primary
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 7 != 3)
        .map(|(_, b)| {
            Bar::new(
                b.timestamp,
                b.open * 2.0,
                b.high * 2.0,
                b.low * 2.0,
                b.close * 2.0,
                b.volume,
            )
        })
        .collect();
    let sym_aligned = align_symbol_bars(&primary, &infy_raw).expect("align");

    let positions: Vec<PositionView> = (0..primary.len())
        .map(|i| match (i / 5) % 3 {
            1 => PositionView {
                side: PositionSide::Long,
                entry_price: 102.0,
                bars_held: ((i % 5) + 1) as u64,
                pnl: primary[i].close - 102.0,
            },
            _ => PositionView::default(),
        })
        .collect();

    // 1. Live bar-by-bar run
    let mut live_exec = Executor::new(&ir).expect("live");
    let mut live_out = Vec::with_capacity(primary.len());
    for i in 0..primary.len() {
        live_out.push(
            live_exec
                .step_with_context(primary[i], positions[i], &[sym_aligned[i]])
                .expect("step"),
        );
    }

    // 2. Chunked run
    let mut chunk_exec = Executor::new(&ir).expect("chunk");
    let mut chunk_out = Vec::with_capacity(primary.len());
    let mut scratch = Vec::new();
    let mut offset = 0usize;
    for chunk_len in [17, 29, 53, primary.len()] {
        if offset >= primary.len() {
            break;
        }
        let end = (offset + chunk_len).min(primary.len());
        scratch.clear();
        chunk_exec
            .run_chunk_with_context_into(
                &primary[offset..end],
                &positions[offset..end],
                &sym_aligned[offset..end],
                &mut scratch,
            )
            .expect("chunk");
        chunk_out.extend_from_slice(&scratch);
        offset = end;
    }

    assert_eq!(live_out, chunk_out);
    assert_eq!(live_exec.snapshot(), chunk_exec.snapshot());

    // 3. Mid-run snapshot + restore across day boundary equals uninterrupted run
    let split = 75;
    let mut part1 = Executor::new(&ir).expect("part1");
    let mut split_out = Vec::with_capacity(primary.len());
    part1
        .run_chunk_with_context_into(
            &primary[..split],
            &positions[..split],
            &sym_aligned[..split],
            &mut split_out,
        )
        .unwrap();
    let snap = part1.snapshot();
    let mut part2 = Executor::new(&ir).expect("part2");
    part2.restore(&snap).unwrap();
    part2
        .run_chunk_with_context_into(
            &primary[split..],
            &positions[split..],
            &sym_aligned[split..],
            &mut split_out,
        )
        .unwrap();
    assert_eq!(live_out, split_out);
    assert_eq!(live_exec.snapshot(), part2.snapshot());
}

#[test]
fn t5_10_timestamp_shift_invariance() {
    let src = "\
state peak:
    init = 0.0
    reset = exit
on enter:
    peak = close
while long:
    peak = max(peak, high)
d_sma = sma(close, 2) @ 1d
h_ema = ema(close, 3) @ 1h
sym_c = symbol(\"INFY\").close @ 1d
entry = close > d_sma and close > h_ema and close > sym_c * 0.4
exit = position.side == \"long\" and close < peak * 0.99
";
    let ir = compile(src);
    let d0 = 1_700_006_400 + 9 * 3600 + 15 * 60;
    let mut bars = Vec::new();
    for day in 0..4 {
        bars.extend(build_day_1m_bars(
            d0 + day * 86_400,
            75,
            100.0 + (day as f64) * 2.0,
        ));
    }
    let sym: Vec<Option<Bar>> = bars
        .iter()
        .enumerate()
        .map(|(i, b)| {
            (i % 5 != 2).then_some(Bar::new(
                b.timestamp,
                b.open * 2.0,
                b.high * 2.0,
                b.low * 2.0,
                b.close * 2.0,
                b.volume,
            ))
        })
        .collect();

    let run_with_shift = |shift_secs: i64, calendar: SessionCalendar| -> Vec<(u8, u8)> {
        let mut exec = Executor::new_with_calendar(&ir, calendar).expect("exec");
        let mut tracker = PositionTracker::new();
        let mut sigs = Vec::with_capacity(bars.len());
        for (i, &b) in bars.iter().enumerate() {
            let mut sb = b;
            sb.timestamp += shift_secs;
            let ssym = sym[i].map(|mut x| {
                x.timestamp += shift_secs;
                x
            });
            let pos = tracker.begin_bar(&sb);
            let out: BarOutput = exec.step_with_context(sb, pos, &[ssym]).expect("step");
            tracker.end_bar(&sb, &out);
            sigs.push((out.entry_signal, out.exit_signal));
        }
        sigs
    };

    let base_sigs = run_with_shift(0, SessionCalendar::utc());
    // 1. Shift by full days / weeks under unchanged default UTC calendar.
    let week_shift_sigs = run_with_shift(70 * 86_400, SessionCalendar::utc());
    assert_eq!(base_sigs, week_shift_sigs);

    // 2. Arbitrary constant shift (e.g. +123_456s) with matching SessionCalendar origin.
    let arb_shift = 123_456_i64;
    let arb_shift_sigs = run_with_shift(arb_shift, SessionCalendar::with_offset(-arb_shift));
    assert_eq!(base_sigs, arb_shift_sigs);
}

#[test]
fn zero_tf_is_guarded_in_bucket_and_rejected_by_executor() {
    let cal = SessionCalendar::utc();
    let _ = cal.bucket(1_700_000_000, Tf::new(0, TfUnit::Minute));
    let _ = cal.bucket(1_700_000_000, Tf::new(0, TfUnit::Week));

    let mut ir = IrGraph::new(Tf::new(1, TfUnit::Minute));
    let zero_node = ir.intern(
        IrOp::Series(SeriesField::Close),
        vec![],
        vec![],
        IrClock::Tf(Tf::new(0, TfUnit::Minute)),
        IrTy::Num,
    );
    let zero = ir.intern(
        IrOp::Const(IrConst::Int(0)),
        vec![],
        vec![],
        IrClock::Constant,
        IrTy::Dec,
    );
    let cmp = ir.intern(
        IrOp::Compare(CmpOp::Gt),
        vec![zero_node, zero],
        vec![],
        IrClock::Tf(Tf::new(0, TfUnit::Minute)),
        IrTy::Bool,
    );
    ir.entry = cmp;
    ir.exit = cmp;
    assert!(matches!(Executor::new(&ir), Err(ExecError::InvalidNode(_))));
}

#[test]
fn non_numeric_bool_state_var_rejected_by_executor() {
    for (c, ty) in [
        (IrConst::Str("x".into()), IrTy::Str),
        (IrConst::Tf(Tf::new(1, TfUnit::Day)), IrTy::Tf),
    ] {
        let mut ir = IrGraph::new(Tf::new(1, TfUnit::Minute));
        let init_node = ir.intern(IrOp::Const(c), vec![], vec![], IrClock::Constant, ty);
        let b_false = ir.intern(
            IrOp::Const(IrConst::Bool(false)),
            vec![],
            vec![],
            IrClock::Constant,
            IrTy::Bool,
        );
        ir.states.push(IrStateVar {
            id: StateId(0),
            name: "bad_state".into(),
            ty,
            init: init_node,
            reset: IrReset::None,
        });
        ir.entry = b_false;
        ir.exit = b_false;
        assert!(matches!(Executor::new(&ir), Err(ExecError::InvalidNode(_))));
    }
}

#[test]
fn align_symbol_bars_rejects_non_monotonic_primary_timestamps() {
    let primary = [
        Bar::new(1_700_000_060, 100.0, 101.0, 99.0, 100.5, 10.0),
        Bar::new(1_700_000_000, 100.5, 101.5, 99.5, 101.0, 10.0),
    ];
    let sym = [Bar::new(1_700_000_060, 200.0, 201.0, 199.0, 200.5, 20.0)];
    let err = align_symbol_bars(&primary, &sym).unwrap_err();
    assert_eq!(
        err,
        ExecError::NonMonotonicTimestamp {
            prev: 1_700_000_060,
            cur: 1_700_000_000,
        }
    );
}

#[test]
fn higher_tf_position_view_uses_completed_bucket_last_pos() {
    let ir = compile("d_held = position.bars_held @ 1d\nentry = d_held >= 5\nexit = false\n");
    let d0 = 1_700_006_400 + 9 * 3600;
    let d1 = d0 + 86_400;
    let b0 = Bar::new(d0, 100.0, 101.0, 99.0, 100.5, 10.0);
    let b1 = Bar::new(d0 + 60, 100.5, 101.5, 99.5, 101.0, 10.0);
    let b2 = Bar::new(d1, 101.0, 102.0, 100.0, 101.5, 10.0);

    let p0 = PositionView {
        side: PositionSide::Long,
        entry_price: 100.0,
        bars_held: 2,
        pnl: 0.5,
    };
    let p1 = PositionView {
        side: PositionSide::Long,
        entry_price: 100.0,
        bars_held: 5,
        pnl: 1.0,
    };
    let p2 = PositionView::default();

    let mut exec = Executor::new(&ir).expect("exec");
    exec.step_with_context(b0, p0, &[]).unwrap();
    exec.step_with_context(b1, p1, &[]).unwrap();
    let snap = exec.snapshot();
    let out = exec.step_with_context(b2, p2, &[]).unwrap();
    assert_eq!(binding_num(&exec, &ir, "d_held"), 5.0);
    assert_eq!(out.entry, TriBool::True);

    // Snapshot before bucket rollover preserves `last_pos`.
    let mut resumed = Executor::new(&ir).expect("resumed");
    resumed.restore(&snap).unwrap();
    let out_resumed = resumed.step_with_context(b2, p2, &[]).unwrap();
    assert_eq!(binding_num(&resumed, &ir, "d_held"), 5.0);
    assert_eq!(out_resumed.entry, TriBool::True);
}

#[test]
fn run_chunk_with_context_reports_expected_total_symbol_count_mismatch() {
    let ir = compile("spy_c = symbol(\"SPY\").close\nentry = close > spy_c\nexit = false\n");
    let mut exec = Executor::new(&ir).expect("exec");
    let bars = [
        Bar::new(1_700_000_000, 100.0, 101.0, 99.0, 100.5, 10.0),
        Bar::new(1_700_000_060, 100.5, 101.5, 99.5, 101.0, 10.0),
        Bar::new(1_700_000_120, 101.0, 102.0, 100.0, 101.5, 10.0),
    ];
    let positions = vec![PositionView::default(); bars.len()];
    let bad_symbols = vec![None; 2];
    let mut out = Vec::new();
    let err = exec
        .run_chunk_with_context_into(&bars, &positions, &bad_symbols, &mut out)
        .unwrap_err();
    assert_eq!(
        err,
        ExecError::SymbolCountMismatch {
            expected: 3,
            got: 2,
        }
    );
}

#[test]
fn nested_coarsest_first_multi_tf_and_weekly_bucketing() {
    // Nested `1d` inside `1h`: on simultaneous `1d` + `1h` boundary, `1d` must complete
    // before `1h` evaluates so `h_with_d` observes the newly completed 2-day SMA.
    let src_nested = "\
h_with_d = (sma(close, 2) @ 1d + close) @ 1h
entry = h_with_d > 0.0
exit = false
";
    let ir_nested = compile(src_nested);
    let mut exec_nested = Executor::new(&ir_nested).expect("exec");

    // 2023-11-15 00:00 UTC = 1_700_006_400
    let day0 = 1_700_006_400;
    let day1 = day0 + 86_400;
    let day2 = day0 + 2 * 86_400;

    let bars = [
        // Day 0: two hours -> day0_close = 12.0
        Bar::new(day0 + 10 * 3600, 10.0, 11.0, 9.0, 10.0, 1.0),
        Bar::new(day0 + 11 * 3600, 12.0, 13.0, 11.0, 12.0, 1.0),
        // Day 1: two hours -> day1_close = 24.0
        Bar::new(day1 + 10 * 3600, 20.0, 21.0, 19.0, 20.0, 1.0),
        Bar::new(day1 + 11 * 3600, 24.0, 25.0, 23.0, 24.0, 1.0),
        // Day 2 boundary crosses BOTH 1d and 1h simultaneously.
        Bar::new(day2 + 10 * 3600, 30.0, 31.0, 29.0, 30.0, 1.0),
    ];

    for &b in &bars[..4] {
        exec_nested.step(b).unwrap();
        assert!(binding_num(&exec_nested, &ir_nested, "h_with_d").is_nan());
    }
    exec_nested.step(bars[4]).unwrap();
    // Completed 1d SMA(2) = (12.0 + 24.0) / 2.0 = 18.0; completed 1h close = 24.0.
    let expected_h_with_d = 18.0 + 24.0;
    assert!((binding_num(&exec_nested, &ir_nested, "h_with_d") - expected_h_with_d).abs() < 1e-12);

    // `TfUnit::Week` bucketing (Monday-Sunday ISO week).
    let ir_week = compile("w_close = close @ 1w\nentry = close > w_close\nexit = false\n");
    let mut exec_week = Executor::new(&ir_week).expect("week exec");
    // 2023-11-17 is Friday (1_700_179_200), +2d Sunday, +3d Monday (new week).
    let fri = 1_700_179_200 + 12 * 3600;
    let sun = fri + 2 * 86_400;
    let mon = fri + 3 * 86_400;

    exec_week
        .step(Bar::new(fri, 100.0, 101.0, 99.0, 101.0, 10.0))
        .unwrap();
    assert!(binding_num(&exec_week, &ir_week, "w_close").is_nan());
    exec_week
        .step(Bar::new(sun, 101.0, 105.0, 100.0, 104.5, 10.0))
        .unwrap();
    assert!(binding_num(&exec_week, &ir_week, "w_close").is_nan());
    exec_week
        .step(Bar::new(mon, 105.0, 106.0, 104.0, 105.5, 10.0))
        .unwrap();
    assert_eq!(binding_num(&exec_week, &ir_week, "w_close"), 104.5);
}

#[test]
fn state_events_while_guards_and_short_position_pnl() {
    let src = "\
state s_enter:
    init = 1.0
    reset = enter
state s_exit_cb:
    init = 0.0
state s_short:
    init = 0.0
state s_flat:
    init = 0.0
state s_cond:
    init = 0.0
on exit:
    s_exit_cb = s_exit_cb + 10.0
while short:
    s_short = s_short + 1.0
    s_enter = s_enter + 5.0
while flat:
    s_flat = s_flat + 1.0
while close > 100.0:
    s_cond = s_cond + 1.0
is_short = position.side == \"short\"
cur_pnl = position.pnl
entry = s_enter > 0.0 and s_exit_cb >= 0.0 and s_short >= 0.0 and s_flat >= 0.0 and s_cond >= 0.0
exit = is_short and cur_pnl >= 5.0
";
    let ir = compile(src);
    let mut exec = Executor::new(&ir).expect("exec");

    let flat = PositionView::default();
    let short_open = PositionView {
        side: PositionSide::Short,
        entry_price: 105.0,
        bars_held: 1,
        pnl: 105.0 - 102.0,
    };
    let short_profit = PositionView {
        side: PositionSide::Short,
        entry_price: 105.0,
        bars_held: 2,
        pnl: 105.0 - 98.0,
    };

    // Bar 0 (flat, close=101 > 100): `while flat` and `while close > 100` run.
    exec.step_with_context(Bar::new(60, 100.0, 102.0, 99.0, 101.0, 10.0), flat, &[])
        .unwrap();
    assert_eq!(exec.state_value("s_flat"), Some(NodeVal::Num(1.0)));
    assert_eq!(exec.state_value("s_cond"), Some(NodeVal::Num(1.0)));
    assert_eq!(exec.state_value("s_short"), Some(NodeVal::Num(0.0)));
    assert_eq!(exec.state_value("s_enter"), Some(NodeVal::Num(1.0)));

    // Bar 1 (enter short, close=102 > 100): `reset = enter` resets `s_enter` to 1.0, then `while short` adds 5.0 -> 6.0.
    let out1 = exec
        .step_with_context(
            Bar::new(120, 104.0, 105.0, 101.0, 102.0, 10.0),
            short_open,
            &[],
        )
        .unwrap();
    assert_eq!(binding_bool(&exec, &ir, "is_short"), TriBool::True);
    assert_eq!(binding_num(&exec, &ir, "cur_pnl"), 3.0);
    assert_eq!(out1.exit_signal, 0);
    assert_eq!(exec.state_value("s_enter"), Some(NodeVal::Num(6.0)));
    assert_eq!(exec.state_value("s_short"), Some(NodeVal::Num(1.0)));
    assert_eq!(exec.state_value("s_flat"), Some(NodeVal::Num(1.0)));
    assert_eq!(exec.state_value("s_cond"), Some(NodeVal::Num(2.0)));

    // Bar 2 (short, close=98 <= 100): short PnL = 7.0 >= 5.0 -> exit_signal = 1; `while close > 100` paused.
    let out2 = exec
        .step_with_context(
            Bar::new(180, 102.0, 103.0, 97.0, 98.0, 10.0),
            short_profit,
            &[],
        )
        .unwrap();
    assert_eq!(binding_num(&exec, &ir, "cur_pnl"), 7.0);
    assert_eq!(out2.exit_signal, 1);
    assert_eq!(exec.state_value("s_enter"), Some(NodeVal::Num(11.0)));
    assert_eq!(exec.state_value("s_short"), Some(NodeVal::Num(2.0)));
    assert_eq!(exec.state_value("s_cond"), Some(NodeVal::Num(2.0)));
    assert_eq!(exec.state_value("s_exit_cb"), Some(NodeVal::Num(0.0)));

    // Bar 3 (exit back to flat, close=99): `on exit:` fires (+10.0), `while flat` fires (+1.0).
    exec.step_with_context(Bar::new(240, 98.0, 100.0, 97.0, 99.0, 10.0), flat, &[])
        .unwrap();
    assert_eq!(exec.state_value("s_exit_cb"), Some(NodeVal::Num(10.0)));
    assert_eq!(exec.state_value("s_flat"), Some(NodeVal::Num(2.0)));
    assert_eq!(exec.state_value("s_short"), Some(NodeVal::Num(2.0)));
}

#[test]
fn phase5_state_def_and_higher_tf_fixes_end_to_end() {
    // Part 1: self-referencing reset, `def` reading `state` across epochs,
    // and higher-TF expression inside `on enter:` block (`dp = high @ 1d`).
    let src = "\
state k:
    init = 0.0
    reset = k >= 2.0
state dp:
    init = 0.0
def add_k(x) = k + x
before_k = add_k(10.0)
on true:
    k = k + 1.0
on enter:
    dp = high @ 1d
after_k = add_k(10.0)
entry = before_k >= 0.0 and after_k >= 0.0 and dp >= 0.0
exit = false
";
    let ir = compile(src);
    let mut exec = Executor::new(&ir).expect("exec");

    let d0 = 1_700_006_400 + 9 * 3600;
    let d1 = d0 + 86_400;
    let flat = PositionView::default();
    let long = PositionView {
        side: PositionSide::Long,
        entry_price: 100.0,
        bars_held: 1,
        pnl: 0.0,
    };

    // Bar 0 (d0, flat): k starts at 0.0 -> before_k = 10.0 -> on true sets k = 1.0 -> after_k = 11.0.
    exec.step_with_context(Bar::new(d0, 100.0, 105.0, 99.0, 100.0, 10.0), flat, &[])
        .unwrap();
    assert_eq!(binding_num(&exec, &ir, "before_k"), 10.0);
    assert_eq!(binding_num(&exec, &ir, "after_k"), 11.0);
    assert_eq!(exec.state_value("dp"), Some(NodeVal::Num(0.0)));

    // Bar 1 (d0, flat): k=1.0 -> reset `k >= 2.0` is false -> before_k=11.0 -> k=2.0 -> after_k=12.0.
    exec.step_with_context(
        Bar::new(d0 + 60, 100.0, 107.0, 99.0, 106.0, 10.0),
        flat,
        &[],
    )
    .unwrap();
    assert_eq!(binding_num(&exec, &ir, "before_k"), 11.0);
    assert_eq!(binding_num(&exec, &ir, "after_k"), 12.0);

    // Bar 2 (d1, enter long): k=2.0 -> reset `k >= 2.0` resets k=0.0 -> before_k=10.0 -> on true sets k=1.0 -> after_k=11.0.
    // Completed day0 high was 107.0, assigned into `dp` on `on enter:`.
    exec.step_with_context(Bar::new(d1, 100.0, 101.0, 93.0, 94.0, 10.0), long, &[])
        .unwrap();
    assert_eq!(binding_num(&exec, &ir, "before_k"), 10.0);
    assert_eq!(binding_num(&exec, &ir, "after_k"), 11.0);
    assert_eq!(exec.state_value("dp"), Some(NodeVal::Num(107.0)));

    // Part 2: `state` variables named `entry` and `exit` drive `BarOutput`.
    let src_state_outputs = "\
state entry:
    init = false
state exit:
    init = false
on close > 105.0:
    entry = true
on close < 95.0:
    exit = true
";
    let ir_out = compile(src_state_outputs);
    let mut exec_out = Executor::new(&ir_out).expect("exec_out");
    let o0 = exec_out
        .step(Bar::new(d0, 100.0, 101.0, 99.0, 100.0, 10.0))
        .unwrap();
    assert_eq!(o0.entry, TriBool::False);
    assert_eq!(o0.exit, TriBool::False);
    let o1 = exec_out
        .step(Bar::new(d0 + 60, 100.0, 107.0, 99.0, 106.0, 10.0))
        .unwrap();
    assert_eq!(o1.entry, TriBool::True);
    assert_eq!(o1.exit, TriBool::False);
    let o2 = exec_out
        .step(Bar::new(d1, 100.0, 101.0, 93.0, 94.0, 10.0))
        .unwrap();
    assert_eq!(o2.entry, TriBool::True);
    assert_eq!(o2.exit, TriBool::True);
}

#[test]
fn phase5_error_paths_monotonicity_invalid_sym_bar_and_align() {
    let ir = compile("spy_c = symbol(\"SPY\").close @ 1d\nentry = close > spy_c\nexit = false\n");
    let mut exec = Executor::new(&ir).expect("exec");

    let b0 = Bar::new(1_700_000_060, 100.0, 101.0, 99.0, 100.5, 10.0);
    let s0 = Some(Bar::new(1_700_000_060, 200.0, 201.0, 199.0, 200.5, 20.0));
    exec.step_with_context(b0, PositionView::default(), &[s0])
        .unwrap();
    let snap_before = format!("{:?}", exec.snapshot());

    // Non-monotonic timestamp in `step_with_context` leaves state untouched.
    let backwards = Bar::new(1_700_000_000, 100.0, 101.0, 99.0, 100.5, 10.0);
    assert_eq!(
        exec.step_with_context(backwards, PositionView::default(), &[s0]),
        Err(ExecError::NonMonotonicTimestamp {
            prev: 1_700_000_060,
            cur: 1_700_000_000,
        })
    );
    assert_eq!(format!("{:?}", exec.snapshot()), snap_before);

    // Invalid secondary `sym_bar` (`high < low`) leaves state untouched.
    let b1 = Bar::new(1_700_000_120, 100.0, 101.0, 99.0, 100.5, 10.0);
    let bad_sym = Some(Bar::new(1_700_000_120, 200.0, 198.0, 199.0, 200.0, 20.0));
    assert!(matches!(
        exec.step_with_context(b1, PositionView::default(), &[bad_sym]),
        Err(ExecError::InvalidBar(_))
    ));
    assert_eq!(format!("{:?}", exec.snapshot()), snap_before);

    // `align_symbol_bars` rejects non-monotonic secondary symbol timestamps.
    let primary = [b0, b1];
    let bad_sym_series = [
        Bar::new(1_700_000_060, 200.0, 201.0, 199.0, 200.5, 20.0),
        Bar::new(1_700_000_000, 200.0, 201.0, 199.0, 200.5, 20.0),
    ];
    assert_eq!(
        align_symbol_bars(&primary, &bad_sym_series),
        Err(ExecError::NonMonotonicTimestamp {
            prev: 1_700_000_060,
            cur: 1_700_000_000,
        })
    );
}
