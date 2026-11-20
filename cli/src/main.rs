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
        println!("algomln run <strategy> [--data DIR] [--tf 1m] [--from DATE] [--to DATE] [--update --master CSV] [--allow-gaps] [--replay FILE] [--live] [--record FILE] [--mode ticker|quote]\nalgomln data <update|status|verify|import|bulk> [options]\nalgomln data bulk --universe nse-eq --tf 1d,1m --from DATE [--to DATE] [--symbols FILE] [--budget N] [--dry-run] --master CSV");
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
            "--skip-bad" | "--debug-invariants" | "--update" | "--allow-gaps" | "--timings" | "--live"
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
            "--replay",
            "--record",
            "--mode",
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
    let from = date("--from")?;
    validate_date_range(from, to)?;
    let range = DateRange {
        from,
        to,
    };
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
    let warmup_bars = opts
        .get("--warmup-bars")
        .map_or(Ok(program.warmup_bars()), |s| {
            s.parse().map_err(|e| format!("invalid --warmup-bars: {e}"))
        })?;
    let mut catalog = algomln_data::CatalogSource::new(
        algomln_data::Store::new(&data),
        data.join("catalog.sqlite"),
    );
    catalog.warmup_bars = warmup_bars;
    catalog.allow_gaps = opts.contains_key("--allow-gaps");

    if opts.contains_key("--update") {
        let master = opts
            .get("--master")
            .ok_or("--update requires --master <api-scrip-master-detailed.csv>")?;
        let rows = dhan_api::parse_scrip_master(std::fs::File::open(master)?)?;
        let fno_count = rows.iter().filter(|r| r.is_fno).count();
        println!(
            "scrip master: {} equities ({} F&O-eligible)",
            rows.len(),
            fno_count
        );
        let db = data.join("catalog.sqlite");
        let mut cat = algomln_data::Catalog::open(&db)?;
        cat.refresh_instruments(&rows, chrono::Utc::now().timestamp())?;
        let to_d = opts
            .get("--to")
            .map(|x| NaiveDate::parse_from_str(x, "%Y-%m-%d"))
            .transpose()?
            .unwrap_or_else(|| chrono::Utc::now().with_timezone(&chrono_tz::Asia::Kolkata).date_naive());
        let from_d = opts
            .get("--from")
            .map(|x| NaiveDate::parse_from_str(x, "%Y-%m-%d"))
            .transpose()?
            .unwrap_or(to_d - chrono::Days::new(365 * 5));
        if from_d > to_d {
            return Err("from date cannot be after to date".into());
        }

        let data_tf = algomln_data::Tf::parse(tf.as_str()).unwrap();
        let mut symbols_with_gaps = Vec::new();
        for symbol in &symbols {
            let gaps = catalog.gaps(symbol, data_tf, from_d, to_d);
            if !gaps.is_empty() {
                symbols_with_gaps.push((symbol, gaps));
            }
        }

        if !symbols_with_gaps.is_empty() {
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
                        data_tf,
                        &instrument,
                        from_d,
                        to_d,
                    )?;
                }
            }
            for (symbol, gaps) in symbols_with_gaps {
                let (id, _) = cat.resolve(symbol)?;
                let instrument = cat.instrument(&id)?;
                let mut updater = algomln_data::Updater {
                    client: Box::new(dhan_api::DhanClient::with_base(token.clone(), &base)?),
                    catalog: algomln_data::Catalog::open(&db)?,
                    store: algomln_data::Store::new(&data),
                    max_retries: 3,
                    margin_seconds: 30,
                    reference_symbol_id: reference_id.clone(),
                    budget_cap: algomln_data::DEFAULT_BUDGET_CAP,
                };
                for (gap_from, gap_to) in gaps {
                    updater.update(&id, data_tf, &instrument, gap_from, gap_to)?;
                }
            }
        }
    }
    let mut legacy = FileSource::new(data.clone());
    legacy.csv_tz = opts.get("--csv-tz").cloned();
    legacy.skip_bad = opts.contains_key("--skip-bad");

    let live_source_holder;
    let src: &dyn algomln::sim::CandleSource = if let Some(replay_path) = opts.get("--replay") {
        let replay = dhan_api::feed::ReplaySource::from_file(replay_path)?;
        let mut sym_map = std::collections::HashMap::new();
        let db = data.join("catalog.sqlite");
        if let Ok(cat) = algomln_data::Catalog::open(&db) {
            for sym in &symbols {
                if let Ok((id_str, _)) = cat.resolve(sym) {
                    let id: i32 = id_str.parse().unwrap_or(0);
                    let is_fno = cat.instrument(&id_str).is_ok();
                    sym_map.insert(sym.clone(), (id, is_fno));
                }
            }
        }
        let mut cat_src = algomln_data::CatalogSource::new(
            algomln_data::Store::new(&data),
            data.join("catalog.sqlite"),
        );
        cat_src.warmup_bars = warmup_bars;
        cat_src.allow_gaps = opts.contains_key("--allow-gaps");
        live_source_holder = Some(
            algomln_data::LiveCandleSource::new(Box::new(replay), sym_map).with_catalog(cat_src),
        );
        live_source_holder.as_ref().unwrap()
    } else if opts.contains_key("--live") {
        let token = std::env::var("DHAN_ACCESS_TOKEN")
            .map_err(|_| "missing DHAN_ACCESS_TOKEN for live feed")?;
        let mut instruments = Vec::new();
        let mut sym_map = std::collections::HashMap::new();
        let db = data.join("catalog.sqlite");
        if let Ok(cat) = algomln_data::Catalog::open(&db) {
            for sym in &symbols {
                if let Ok((id_str, _)) = cat.resolve(sym) {
                    let id: i32 = id_str.parse().unwrap_or(0);
                    let is_fno = cat.instrument(&id_str).is_ok();
                    sym_map.insert(sym.clone(), (id, is_fno));
                    instruments.push(dhan_api::feed::InstrumentSubscription::nse_eq(&id_str));
                }
            }
        }
        let mode = match opts.get("--mode").map(String::as_str).unwrap_or("ticker") {
            "ticker" => dhan_api::feed::RequestCode::Ticker,
            "quote" => dhan_api::feed::RequestCode::Quote,
            other => return Err(format!("invalid --mode '{other}': expected 'ticker' or 'quote'").into()),
        };
        let mut feed_cfg = dhan_api::feed::FeedConfig::new(token, instruments)
            .with_request_code(mode);
        if let Some(rec) = opts.get("--record") {
            feed_cfg = feed_cfg.with_record_path(rec);
        }
        let live = dhan_api::feed::LiveSource::connect(feed_cfg)?;
        let mut cat_src = algomln_data::CatalogSource::new(
            algomln_data::Store::new(&data),
            data.join("catalog.sqlite"),
        );
        cat_src.warmup_bars = warmup_bars;
        cat_src.allow_gaps = opts.contains_key("--allow-gaps");
        live_source_holder = Some(
            algomln_data::LiveCandleSource::new(Box::new(live), sym_map).with_catalog(cat_src),
        );
        live_source_holder.as_ref().unwrap()
    } else if opts.contains_key("--csv-tz") || opts.contains_key("--skip-bad") {
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
    let fp_start = src.fingerprint(&symbols, config.timeframe, config.range);
    println!("dataset fingerprint: {fp_start}");
    if let Ok(info) = catalog.resolved_info(&symbols, config.timeframe, config.range) {
        let from_str = info
            .resolved_from
            .map(|t| {
                chrono::DateTime::from_timestamp(t, 0)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| t.to_string())
            })
            .unwrap_or_else(|| "none".into());
        let to_str = info
            .resolved_to
            .map(|t| {
                chrono::DateTime::from_timestamp(t, 0)
                    .map(|dt| dt.to_rfc3339())
                    .unwrap_or_else(|| t.to_string())
            })
            .unwrap_or_else(|| "none".into());
        let warmup_str = info
            .warmup_start
            .map(|d| d.to_string())
            .unwrap_or_else(|| "none".into());
        println!("resolved range: {from_str} ..= {to_str}");
        println!("warmup start: {warmup_str}");
        println!(
            "symbols: total={}, F&O={}, non-F&O={}",
            symbols.len(),
            info.fno_count,
            info.non_fno_count
        );
    }
    let tf = config.timeframe;
    let range = config.range;
    let mut result = sim::run(&program, src, config, Some(symbols.clone()))?;
    let fp_end = src.fingerprint(&symbols, tf, range);
    println!("dataset fingerprint (end): {fp_end}");
    if fp_start != fp_end {
        eprintln!("WARNING: dataset fingerprint changed mid-run! (data update detected)");
        if let Some(obj) = result.manifest.as_object_mut() {
            obj.insert("fingerprint_changed".into(), serde_json::Value::Bool(true));
        }
    }
    let out_start = std::time::Instant::now();
    sim::write_results(
        opts.get("--out")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("run-output")),
        &result,
    )?;
    let out_time = out_start.elapsed();
    if opts.contains_key("--timings") {
        println!("timings:");
        println!("  load/merge:        {:?}", result.timings.load_merge);
        println!("  engine+indicators: {:?}", result.timings.engine_indicators);
        println!("  trader:            {:?}", result.timings.trader);
        println!("  output writing:    {:?}", out_time);
    }
    Ok(())
}

fn validate_date_range(from: Option<i64>, to: Option<i64>) -> Result<(), String> {
    if let (Some(f), Some(t)) = (from, to) {
        if f > t {
            return Err("from date cannot be after to date".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inverted_date_range_is_rejected() {
        // from: 2024-05-10 00:00:00 IST (ts: 1715281200)
        // to:   2024-05-01 23:59:59 IST (ts: 1714589999)
        let from = Some(1715281200);
        let to = Some(1714589999);
        let res = validate_date_range(from, to);
        assert!(res.is_err());
        assert_eq!(res.unwrap_err(), "from date cannot be after to date");
    }

    #[test]
    fn test_valid_date_range_is_accepted() {
        let from = Some(1714500000);
        let to = Some(1715281200);
        assert!(validate_date_range(from, to).is_ok());
        assert!(validate_date_range(None, to).is_ok());
        assert!(validate_date_range(from, None).is_ok());
        assert!(validate_date_range(None, None).is_ok());
    }
}
