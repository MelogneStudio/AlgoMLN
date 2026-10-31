use algomln::sim::{self, DateRange, FeeModel, FileSource, GapPolicy, RunConfig, Timeframe};
mod data_cli;
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
    let mut args = env::args().skip(1);
    let command = args.next().ok_or("usage: algomln run|data ...")?;
    if command == "--help" || command == "-h" {
        println!("algomln run <strategy> [--data DIR] [--tf 1m] [--from DATE] [--to DATE] [--update --master CSV] [--allow-gaps]\nalgomln data <update|status|verify|import|bulk> [options]\nalgomln data bulk --universe nse-eq --tf 1d,1m --from DATE [--to DATE] [--symbols FILE] [--budget N] [--dry-run] --master CSV");
        return Ok(());
    }
    if command == "data" {
        return data_cli::main_entry(args.collect());
    }
    if command != "run" {
        return Err(
            "usage: algomln run <file.algomln> ... | algomln data <update|status|verify|import|bulk>"
                .into(),
        );
    }
    let strategy = PathBuf::from(args.next().ok_or("missing strategy file")?);
    let mut opts = BTreeMap::new();
    while let Some(k) = args.next() {
        if !k.starts_with("--") {
            return Err(format!("unexpected argument {k}").into());
        }
        if matches!(
            k.as_str(),
            "--skip-bad" | "--debug-invariants" | "--update" | "--allow-gaps"
        ) {
            opts.insert(k, "true".into());
            continue;
        }
        if ![
            "--data",
            "--master",
            "--reference",
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
        opts.insert(
            k.clone(),
            args.next()
                .ok_or_else(|| format!("missing value for {k}"))?,
        );
    }
    let data = PathBuf::from(opts.get("--data").map(String::as_str).unwrap_or("data"));
    let tf = Timeframe::parse(opts.get("--tf").map(String::as_str).unwrap_or("1m"))
        .ok_or("invalid --tf")?;
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
    let code = std::fs::read_to_string(&strategy)?;
    let program = algomln::compile(&code).map_err(|e| format!("strategy compile failed: {e}"))?;
    let to = date("--to")?.map(|t| t.saturating_add(86_399));
    let mut range = DateRange {
        from: date("--from")?,
        to,
    };
    if let Some(from) = range.from {
        let daily = matches!(tf, Timeframe::D1);
        let warm = program.warmup_bars() as i64;
        let sessions = (warm + if daily { 1 } else { 359 }) / if daily { 1 } else { 360 };
        let extra = sessions.saturating_mul(86_400 * 7 / 5 + 86_400);
        range.from = Some(from.saturating_sub(extra));
    }
    let symbols: Vec<String> = opts
        .get("--symbols")
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_else(|| program.symbols().into_iter().map(str::to_owned).collect());
    if opts.contains_key("--update") {
        let master = opts
            .get("--master")
            .ok_or("--update requires --master <scrip-master.csv>")?;
        let rows = dhan_api::parse_scrip_master(std::fs::File::open(master)?)?;
        let db = data.join("catalog.sqlite");
        let mut cat = algomln_data::Catalog::open(&db)?;
        cat.refresh_instruments(&rows, chrono::Utc::now().timestamp())?;
        let to = opts
            .get("--to")
            .map(|x| NaiveDate::parse_from_str(x, "%Y-%m-%d"))
            .transpose()?
            .unwrap_or_else(|| chrono::Utc::now().date_naive());
        let from = opts
            .get("--from")
            .map(|x| NaiveDate::parse_from_str(x, "%Y-%m-%d"))
            .transpose()?
            .unwrap_or(to - chrono::Days::new(365 * 5));
        let base =
            std::env::var("DHAN_API_BASE").unwrap_or_else(|_| "https://api.dhan.co/v2".into());
        let token = std::sync::Arc::new(dhan_api::KeychainEnvTokenProvider::default());
        let reference_id = cat
            .resolve(
                opts.get("--reference")
                    .map(String::as_str)
                    .unwrap_or("RELIANCE"),
            )
            .map(|x| x.0)
            .ok();
        if let Some(reference_id) = reference_id.as_deref() {
            if !symbols
                .iter()
                .any(|s| cat.resolve(s).is_ok_and(|(id, _)| id == reference_id))
            {
                let instrument = cat.instrument(reference_id)?;
                algomln_data::Updater {
                    client: Box::new(dhan_api::DhanClient::with_base(token.clone(), &base)?),
                    catalog: algomln_data::Catalog::open(&db)?,
                    store: algomln_data::Store::new(&data),
                    max_retries: 3,
                    margin_seconds: 30,
                    reference_symbol_id: Some(reference_id.to_owned()),
                    budget_cap: algomln_data::DEFAULT_BUDGET_CAP,
                }
                .update(
                    reference_id,
                    algomln_data::Tf::parse(tf.as_str()).unwrap(),
                    &instrument,
                    from,
                    to,
                )?;
            }
        }
        for symbol in &symbols {
            let (id, _) = cat.resolve(symbol)?;
            let instrument = cat.instrument(&id)?;
            algomln_data::Updater {
                client: Box::new(dhan_api::DhanClient::with_base(token.clone(), &base)?),
                catalog: algomln_data::Catalog::open(&db)?,
                store: algomln_data::Store::new(&data),
                max_retries: 3,
                margin_seconds: 30,
                reference_symbol_id: reference_id.clone(),
                budget_cap: algomln_data::DEFAULT_BUDGET_CAP,
            }
            .update(
                &id,
                algomln_data::Tf::parse(tf.as_str()).unwrap(),
                &instrument,
                from,
                to,
            )?;
        }
    }
    let mut legacy = FileSource::new(data.clone());
    legacy.csv_tz = opts.get("--csv-tz").cloned();
    legacy.skip_bad = opts.contains_key("--skip-bad");
    let mut catalog = algomln_data::CatalogSource::new(
        algomln_data::Store::new(&data),
        data.join("catalog.sqlite"),
    );
    catalog.allow_gaps = opts.contains_key("--allow-gaps");
    let src: &dyn algomln::sim::CandleSource =
        if opts.contains_key("--csv-tz") || opts.contains_key("--skip-bad") {
            &legacy
        } else {
            &catalog
        };
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
    let partial = opts
        .get("--partial-fills")
        .map(|s| s.parse::<f64>())
        .transpose()?;
    if partial.is_some_and(|p| !p.is_finite() || p <= 0.0 || p > 100.0) {
        return Err("--partial-fills must be in (0, 100]".into());
    }
    let latency = opts.get("--latency").map_or(Ok(1), |s| {
        s.parse::<u64>()
            .map_err(|e| format!("invalid --latency: {e}"))
    })?;
    if latency == 0 {
        return Err("--latency must be at least 1 candle".into());
    }
    let slip = number("--slippage-bps", 0.0)?;
    if !slip.is_finite() || !(0.0..10000.0).contains(&slip) {
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
        slippage_bps: slip,
        fee_model: fee,
        partial_fills_pct: partial,
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
    let result = sim::run(&program, src, config, Some(symbols))?;
    println!(
        "dataset fingerprint: {}",
        result.manifest["source_fingerprint"]
    );
    sim::write_results(
        opts.get("--out")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("run-output")),
        &result,
    )?;
    Ok(())
}
