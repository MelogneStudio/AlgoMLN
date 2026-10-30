use algomln::sim::{self, DateRange, FeeModel, FileSource, GapPolicy, RunConfig, Timeframe};
use chrono::{FixedOffset, NaiveDate, TimeZone};
use std::{collections::BTreeMap, env, path::PathBuf, process::ExitCode};

fn main() -> ExitCode {
    match run_cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let mut a = env::args().skip(1);
    if a.next().as_deref() != Some("run") {
        return Err(
            "usage: algomln run <file.algomln> --data <dir> --tf <1m|5m|15m|25m|60m|1d> [options]"
                .into(),
        );
    }
    let strategy = PathBuf::from(a.next().ok_or("missing strategy file")?);
    let mut opts = BTreeMap::new();
    while let Some(k) = a.next() {
        if !k.starts_with("--") {
            return Err(format!("unexpected argument {k}").into());
        }
        if k == "--skip-bad" || k == "--debug-invariants" {
            opts.insert(k, "true".into());
            continue;
        }
        if ![
            "--data",
            "--tf",
            "--symbols",
            "--from",
            "--to",
            "--cash",
            "--warmup-bars",
            "--latency",
            "--slippage-bps",
            "--fee-bps",
            "--partial-fills",
            "--gap-policy",
            "--csv-tz",
            "--out",
        ]
        .contains(&k.as_str())
        {
            return Err(format!("unknown option {k}").into());
        }
        let v = a.next().ok_or_else(|| format!("missing value for {k}"))?;
        opts.insert(k, v);
    }
    let data = PathBuf::from(opts.get("--data").ok_or("--data is required")?);
    let tf = Timeframe::parse(opts.get("--tf").ok_or("--tf is required")?).ok_or("invalid --tf")?;
    let date = |key: &str| -> Result<Option<i64>, Box<dyn std::error::Error>> {
        let Some(s) = opts.get(key) else {
            return Ok(None);
        };
        let d = NaiveDate::parse_from_str(s, "%Y-%m-%d")?;
        let dt = d.and_hms_opt(0, 0, 0).unwrap();
        Ok(Some(
            FixedOffset::east_opt(19800)
                .unwrap()
                .from_local_datetime(&dt)
                .single()
                .unwrap()
                .timestamp(),
        ))
    };
    let to = date("--to")?.map(|t| t.saturating_add(86_399));
    let range = DateRange {
        from: date("--from")?,
        to,
    };
    let mut src = {
        let mut x = FileSource::new(data);
        x.csv_tz = opts.get("--csv-tz").cloned();
        x.skip_bad = opts.contains_key("--skip-bad");
        x
    };
    let code = std::fs::read_to_string(&strategy)?;
    let program = algomln::compile(&code).map_err(|e| format!("strategy compile failed: {e}"))?;
    let symbols = opts.get("--symbols").map(|s| {
        s.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    });
    let number = |key: &str, default: f64| {
        opts.get(key).map_or(Ok(default), |s| {
            s.parse().map_err(|e| format!("invalid {key}: {e}"))
        })
    };
    let fee = FeeModel {
        bps: number("--fee-bps", 0.0)?,
        flat_per_order: 0.0,
        min_per_order: 0.0,
    };
    if fee.bps < 0.0 || !fee.bps.is_finite() {
        return Err("--fee-bps must be finite and non-negative".into());
    }
    let partial_fills_pct = opts
        .get("--partial-fills")
        .map(|s| s.parse::<f64>())
        .transpose()?;
    if partial_fills_pct.is_some_and(|p| !p.is_finite() || p <= 0.0 || p > 100.0) {
        return Err("--partial-fills must be in (0, 100]".into());
    }
    let latency = opts.get("--latency").map_or(Ok(1), |s| {
        s.parse::<u64>()
            .map_err(|e| format!("invalid --latency: {e}"))
    })?;
    if latency == 0 {
        return Err("--latency must be at least 1 candle".into());
    }
    let slippage_bps = number("--slippage-bps", 0.0)?;
    if !slippage_bps.is_finite() || !(0.0..10000.0).contains(&slippage_bps) {
        return Err("--slippage-bps must be in [0, 10000)".into());
    }
    let config = RunConfig {
        timeframe: tf,
        range,
        initial_cash: number("--cash", 100_000.0)?,
        warmup_bars: opts
            .get("--warmup-bars")
            .map_or(Ok(program.warmup_bars()), |s| {
                s.parse().map_err(|e| format!("invalid --warmup-bars: {e}"))
            })?,
        latency,
        slippage_bps,
        fee_model: fee,
        partial_fills_pct,
        gap_policy: match opts
            .get("--gap-policy")
            .map(String::as_str)
            .unwrap_or("none")
        {
            "none" => GapPolicy::None,
            "fill" => GapPolicy::FillFlatInSession,
            x => return Err(format!("invalid --gap-policy {x:?}").into()),
        },
        debug_invariants: opts.contains_key("--debug-invariants"),
    };
    src.gap_policy = config.gap_policy;
    let result = sim::run(&program, &src, config, symbols)?;
    sim::write_results(
        opts.get("--out")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("run-output")),
        &result,
    )?;
    Ok(())
}
