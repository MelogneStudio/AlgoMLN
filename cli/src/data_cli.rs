use algomln_data::{
    Bar, BulkConfig, BulkDownloader, Catalog, Store, Tf, Updater, DEFAULT_BUDGET_CAP,
};
use chrono::{Datelike, NaiveDate, Utc};
use dhan_api::{parse_scrip_master, DhanClient, KeychainEnvTokenProvider};
use std::{
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
};

pub fn main_entry(args: Vec<String>) -> Result<(), Box<dyn std::error::Error>> {
    let Some(sub) = args.first().cloned() else {
        return Err("usage: algomln data <status|verify|update|import|bulk> [options]".into());
    };
    let mut opts = std::collections::BTreeMap::new();
    let mut it = args.into_iter().skip(1);
    while let Some(k) = it.next() {
        if !k.starts_with("--") {
            return Err(format!("unexpected argument {k}").into());
        }
        opts.insert(
            k.clone(),
            it.next().ok_or_else(|| format!("missing value for {k}"))?,
        );
    }
    let root = PathBuf::from(opts.get("--data").map(String::as_str).unwrap_or("data"));
    let db = root.join("catalog.sqlite");
    let mut cat = Catalog::open(&db)?;
    let store = Store::new(&root);
    match sub.as_str() {
        "status" => {
            println!("catalog: {}", db.display());
            for series in cat.status_rows()? {
                println!(
                    "{} {}: {} bars",
                    series.symbol, series.timeframe, series.bars
                );
                for range in series.coverage {
                    println!("  {}..{} {}", range.from, range.to, range.kind);
                }
            }
            for (kind, detail) in cat.quality_rows()? {
                println!("quality {kind}: {detail}");
            }
        }
        "verify" => {
            store.reconcile(&mut cat)?;
            println!("catalog reconciled")
        }
        "update" => {
            let master = opts
                .get("--master")
                .ok_or("data update requires --master <scrip-master.csv>")?;
            let rows = parse_scrip_master(std::fs::File::open(master)?)?;
            cat.refresh_instruments(&rows, Utc::now().timestamp())?;
            let symbols = opts
                .get("--symbols")
                .ok_or("data update requires --symbols A,B")?;
            let tf = Tf::parse(opts.get("--tf").map(String::as_str).unwrap_or("1m"))
                .ok_or("invalid timeframe")?;
            let to = opts
                .get("--to")
                .map(|x| NaiveDate::parse_from_str(x, "%Y-%m-%d"))
                .transpose()?
                .unwrap_or_else(|| Utc::now().date_naive());
            let from = opts
                .get("--from")
                .map(|x| NaiveDate::parse_from_str(x, "%Y-%m-%d"))
                .transpose()?
                .unwrap_or(to - chrono::Days::new(365 * 5));
            let (client, base) = client()?;
            let mut did_work = false;
            for symbol in symbols.split(',').map(str::trim).filter(|x| !x.is_empty()) {
                let (id, _) = cat.resolve(symbol)?;
                let instrument = cat.instrument(&id)?;
                let reference_id = cat
                    .resolve(
                        opts.get("--reference")
                            .map(String::as_str)
                            .unwrap_or("RELIANCE"),
                    )
                    .map(|x| x.0)
                    .ok();
                let mut updater = Updater {
                    client: Box::new(client.clone_with_base(&base)?),
                    catalog: Catalog::open(&db)?,
                    store: Store::new(&root),
                    max_retries: 3,
                    margin_seconds: 30,
                    reference_symbol_id: reference_id,
                    budget_cap: algomln_data::DEFAULT_BUDGET_CAP,
                };
                did_work |= updater.update(&id, tf, &instrument, from, to)?;
            }
            if did_work {
                println!("updated {} for {}", tf.as_str(), symbols)
            } else {
                println!("already up to date: {} {}", symbols, tf.as_str())
            }
        }
        "import" => {
            let symbol = opts
                .get("--symbol")
                .ok_or("data import requires --symbol")?;
            let file = PathBuf::from(opts.get("--file").ok_or("data import requires --file")?);
            let tf = Tf::parse(opts.get("--tf").map(String::as_str).unwrap_or("1m"))
                .ok_or("invalid timeframe")?;
            let (id, _) = cat.resolve(symbol)?;
            let bars = import_file(&file, opts.get("--csv-tz").map(String::as_str))?;
            if bars.iter().any(|b| !algomln_data::validate_bar(b)) {
                return Err("import contains invalid bars".into());
            }
            let parts = store.write_partitions(&id, tf, &bars)?;
            let sid = cat.series_for_import(&id, tf)?;
            for part in &parts {
                cat.record_partition(sid, &store, part)?;
            }
            println!("imported {} bars for {symbol}", bars.len())
        }
        "bulk" => {
            if opts
                .get("--universe")
                .map(String::as_str)
                .unwrap_or("nse-eq")
                != "nse-eq"
            {
                return Err("data bulk only supports --universe nse-eq".into());
            }
            let master = opts
                .get("--master")
                .ok_or("data bulk requires --master <scrip-master.csv>")?;
            let rows = parse_scrip_master(std::fs::File::open(master)?)?;
            cat.refresh_instruments(&rows, Utc::now().timestamp())?;
            let tfs = opts
                .get("--tf")
                .map(String::as_str)
                .unwrap_or("1d,1m")
                .split(',')
                .map(str::trim)
                .filter(|x| !x.is_empty())
                .map(|x| Tf::parse(x).ok_or_else(|| format!("invalid timeframe {x}")))
                .collect::<Result<Vec<_>, _>>()?;
            if tfs.is_empty() {
                return Err("data bulk requires at least one --tf".into());
            }
            // 1d first for all symbols, then 1m (spec 4), regardless of input order.
            let mut tfs = tfs;
            tfs.sort_by_key(|t| if *t == Tf::D1 { 0 } else { 1 });
            let to = opts
                .get("--to")
                .map(|x| NaiveDate::parse_from_str(x, "%Y-%m-%d"))
                .transpose()?
                .unwrap_or_else(last_completed_session);
            let from = opts
                .get("--from")
                .map(|x| NaiveDate::parse_from_str(x, "%Y-%m-%d"))
                .transpose()?
                .ok_or("data bulk requires --from <YYYY-MM-DD>")?;
            let budget_cap = opts
                .get("--budget")
                .map(|x| x.parse::<u64>())
                .transpose()
                .map_err(|e| format!("invalid --budget: {e}"))?
                .unwrap_or(DEFAULT_BUDGET_CAP);
            let priority_symbols = match opts.get("--symbols") {
                Some(path) => std::fs::read_to_string(path)?
                    .lines()
                    .map(str::trim)
                    .filter(|x| !x.is_empty())
                    .map(str::to_owned)
                    .collect(),
                None => Vec::new(),
            };
            let reference_symbol_id = cat
                .resolve(
                    opts.get("--reference")
                        .map(String::as_str)
                        .unwrap_or("RELIANCE"),
                )
                .map(|x| x.0)
                .ok();
            // Up to N symbols fetched in parallel (spec 6). Defaults to 4 to
            // overlap request latency; the shared limiter still spaces request
            // starts >=250 ms apart. `--concurrency 1` forces the serial path
            // (exact budget/auth accounting).
            let concurrency = opts
                .get("--concurrency")
                .map(|x| x.parse::<usize>())
                .transpose()
                .map_err(|e| format!("invalid --concurrency: {e}"))?
                .unwrap_or(4)
                .max(1);
            let mut config = BulkConfig::new(from, to, tfs);
            config.budget_cap = budget_cap;
            config.priority_symbols = priority_symbols;
            config.reference_symbol_id = reference_symbol_id;
            config.concurrency = concurrency;
            let db = db.clone();
            let root = root.clone();
            let (client, base) = client()?;
            let make_client = move || {
                Box::new(
                    client
                        .clone_with_base(&base)
                        .expect("clone Dhan client for bulk symbol"),
                ) as Box<dyn algomln_data::HistoryApi>
            };
            let stop = Arc::new(AtomicBool::new(false));
            // Ctrl-C flips the shared stop flag (spec 7). The downloader checks
            // it between symbols, lets the in-flight request finish, and persists
            // budget + catalog before returning. The handler only sets the flag,
            // so repeated Ctrl-C is idempotent (no forced mid-write abort).
            {
                let stop = stop.clone();
                if let Err(e) = ctrlc::set_handler(move || {
                    eprintln!(
                        "\ninterrupt received; finishing the in-flight request and saving progress..."
                    );
                    stop.store(true, std::sync::atomic::Ordering::SeqCst);
                }) {
                    eprintln!("warning: could not install Ctrl-C handler: {e}");
                }
            }
            let bulk = BulkDownloader {
                make_client,
                catalog_path: db,
                store_root: root,
                config,
                stop,
            };
            if opts.contains_key("--dry-run") {
                let report = bulk.dry_run()?;
                let remaining = cat.budget_used(&Utc::now().date_naive().to_string())?;
                let budget_left = budget_cap.saturating_sub(remaining);
                println!("dry-run: {} symbols", report.symbols_total);
                let mut total = 0usize;
                for (tf, n) in &report.planned {
                    let minutes = (*n as f64) / 4.0 / 60.0;
                    println!("  {tf}: {n} requests, est {minutes:.1} min at 4 req/s");
                    total += *n;
                }
                let fits = (total as u64) <= budget_left;
                println!(
                    "  total {total} requests; remaining budget today {budget_left}/{budget_cap}; fits today: {}",
                    if fits { "yes" } else { "no" }
                );
                return Ok(());
            }
            let report = bulk.run(|p| {
                eprint!("\r{}", p.line());
            })?;
            eprintln!();
            println!(
                "bulk finished: {}/{} symbols | requests used {} | failures {} | budget {}/{}",
                report.symbols_completed,
                report.symbols_total,
                report.requests_used,
                report.failures,
                cat.budget_used(&Utc::now().date_naive().to_string())?,
                budget_cap,
            );
            let fp = cat.catalog_fingerprint()?;
            println!("catalog fingerprint: {fp}");
            if report.interrupted {
                println!("interrupted; progress is saved, rerun to continue");
            }
            if report.budget_reached {
                let used = cat.budget_used(&Utc::now().date_naive().to_string())?;
                return Err(format!(
                    "daily budget reached ({used}/{budget_cap}); rerun tomorrow, progress is saved"
                )
                .into());
            }
        }
        _ => return Err(format!("unknown data command {sub}").into()),
    }
    Ok(())
}
/// Last completed NSE session, excluding today (spec 2/10: today's session is
/// excluded until the daily-close settle time is verified). Weekends roll back
/// to Friday; exchange holidays are not modelled here.
fn last_completed_session() -> NaiveDate {
    let mut d = Utc::now().date_naive() - chrono::Days::new(1);
    while matches!(
        d.weekday(),
        chrono::Weekday::Sat | chrono::Weekday::Sun
    ) {
        d = d - chrono::Days::new(1);
    }
    d
}
fn client() -> Result<(DhanClient, String), Box<dyn std::error::Error>> {
    let base = std::env::var("DHAN_API_BASE").unwrap_or_else(|_| "https://api.dhan.co/v2".into());
    Ok((
        DhanClient::with_base(Arc::new(KeychainEnvTokenProvider::default()), &base)?,
        base,
    ))
}
fn import_file(
    path: &std::path::Path,
    tz: Option<&str>,
) -> Result<Vec<Bar>, Box<dyn std::error::Error>> {
    if path
        .extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("parquet"))
    {
        return Ok(algomln_data::read_partition(path)?);
    }
    let mut rdr = csv::Reader::from_path(path)?;
    let h = rdr.headers()?.clone();
    let idx = |n: &str| {
        h.iter()
            .position(|x| x.eq_ignore_ascii_case(n))
            .ok_or_else(|| format!("CSV missing {n}"))
    };
    let (ts, o, hi, lo, c, v) = (
        idx("ts")?,
        idx("open")?,
        idx("high")?,
        idx("low")?,
        idx("close")?,
        idx("volume")?,
    );
    let mut out = Vec::new();
    for row in rdr.records() {
        let r = row?;
        let raw = &r[ts];
        let stamp = if let Ok(i) = raw.parse::<i64>() {
            i
        } else if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {
            dt.timestamp()
        } else {
            let naive = chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S")?;
            let zone = tz.unwrap_or("Asia/Kolkata").parse::<chrono_tz::Tz>()?;
            naive
                .and_local_timezone(zone)
                .single()
                .ok_or("ambiguous local CSV timestamp")?
                .timestamp()
        };
        out.push(Bar {
            ts: stamp,
            open: r[o].parse()?,
            high: r[hi].parse()?,
            low: r[lo].parse()?,
            close: r[c].parse()?,
            volume: r[v].parse()?,
        });
    }
    Ok(out)
}
