use crate::catalog::Catalog;
use crate::error::DataError;
use crate::history::HistoryApi;
use crate::store::Store;
use crate::types::Tf;
use crate::updater::{plan_requests, Updater, DEFAULT_BUDGET_CAP};
use chrono::NaiveDate;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Configuration for a `data bulk` run.
#[derive(Clone, Debug)]
pub struct BulkConfig {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Timeframes to download, in processing order. When both are present the
    /// caller orders 1d before 1m (spec 4).
    pub tfs: Vec<Tf>,
    pub budget_cap: u64,
    /// Priority trading symbols, processed first in the given order (spec 4).
    pub priority_symbols: Vec<String>,
    pub margin_seconds: i64,
    pub max_retries: u32,
    pub reference_symbol_id: Option<String>,
    /// Number of calendar days to wait before retrying a recorded failure.
    pub failure_backoff_days: u64,
    /// Maximum number of symbols fetched in parallel within a timeframe (spec 6).
    /// `1` keeps the fully serial path; higher values fan out across symbols while
    /// the shared `Limiter` still spaces request *starts* >=250 ms apart. The
    /// library default is serial; the CLI opts into 4. Exact budget/auth
    /// accounting is a serial-only guarantee (see [`BulkDownloader::run`]).
    pub concurrency: usize,
}

impl BulkConfig {
    pub fn new(from: NaiveDate, to: NaiveDate, tfs: Vec<Tf>) -> Self {
        Self {
            from,
            to,
            tfs,
            budget_cap: DEFAULT_BUDGET_CAP,
            priority_symbols: Vec::new(),
            margin_seconds: 30,
            max_retries: 3,
            reference_symbol_id: None,
            failure_backoff_days: 1,
            concurrency: 1,
        }
    }
}

/// Outcome of a bulk run (or dry run).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BulkReport {
    pub symbols_completed: usize,
    pub symbols_total: usize,
    pub requests_used: u64,
    pub failures: u64,
    /// Planned request count per timeframe, populated on `--dry-run`.
    pub planned: Vec<(String, usize)>,
    /// True when the run stopped because the daily budget was reached.
    pub budget_reached: bool,
    /// True when the run stopped because of a Ctrl-C style interrupt.
    pub interrupted: bool,
}

/// Drives a full-universe download, reusing [`Updater`] per symbol/tf so request
/// chunking, retries, 5-year fallback, overlap handling and crash-safe writes
/// all come from the shared planner. The downloader owns only ordering, the
/// persisted budget hard-stop, resume and progress.
pub struct BulkDownloader<F: Fn() -> Box<dyn HistoryApi>> {
    /// Factory that yields a fresh client per symbol so the shared rate limiter
    /// (250 ms spacing, DH-904 backoff) is reused without aliasing one updater.
    pub make_client: F,
    pub catalog_path: PathBuf,
    pub store_root: PathBuf,
    pub config: BulkConfig,
    /// Set from a Ctrl-C handler. Checked between symbols; the in-flight symbol
    /// is allowed to finish (spec 7).
    pub stop: Arc<std::sync::atomic::AtomicBool>,
}

impl<F: Fn() -> Box<dyn HistoryApi> + Sync> BulkDownloader<F> {
    fn today(&self) -> String {
        chrono::Utc::now().date_naive().to_string()
    }

    /// Resolve the processing order: priority symbols first (in file order),
    /// then every other instrument in security-id order, de-duplicated.
    pub(crate) fn order(&self, cat: &Catalog) -> Result<Vec<(String, String, String)>, DataError> {
        let all = cat.instruments_in_order()?;
        let by_id: HashMap<String, (String, String)> = all
            .iter()
            .map(|(id, sym, inst)| (id.clone(), (sym.clone(), inst.clone())))
            .collect();
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for sym in &self.config.priority_symbols {
            let (id, _) = cat.resolve(sym)?;
            if seen.insert(id.clone()) {
                if let Some((symbol, inst)) = by_id.get(&id) {
                    out.push((id.clone(), symbol.clone(), inst.clone()));
                }
            }
        }
        for (id, sym, inst) in all {
            if seen.insert(id.clone()) {
                out.push((id, sym, inst));
            }
        }
        Ok(out)
    }

    /// Compute the planned request count per timeframe without sending anything.
    pub fn dry_run(&self) -> Result<BulkReport, DataError> {
        let cat = Catalog::open(&self.catalog_path)?;
        let order = self.order(&cat)?;
        let mut report = BulkReport {
            symbols_total: order.len(),
            ..Default::default()
        };
        for tf in &self.config.tfs {
            let mut total = 0usize;
            for (id, _, _) in &order {
                total += plan_requests(&cat, id, *tf, self.config.from, self.config.to)?;
            }
            report.planned.push((tf.as_str().to_string(), total));
        }
        Ok(report)
    }

    /// Run the download. Processes 1d for every symbol first, then 1m (per the
    /// order in `config.tfs`), committing after each symbol so an interrupt
    /// leaves whole symbols behind. Returns the final report.
    ///
    /// Dispatches to the serial path (`config.concurrency <= 1`) or the
    /// across-symbols concurrent path (`> 1`). The serial path's exact budget and
    /// immediate-auth-abort behaviour is a serial-only guarantee; see
    /// [`run_concurrent`](Self::run_concurrent) for the relaxed concurrent
    /// semantics.
    pub fn run<P: FnMut(&Progress) + Send>(&self, progress: P) -> Result<BulkReport, DataError> {
        if self.config.concurrency > 1 {
            self.run_concurrent(progress)
        } else {
            self.run_serial(progress)
        }
    }

    /// Fully serial download: one symbol at a time, one SQLite connection in
    /// flight. This is the reference implementation; the concurrent path must
    /// produce the same catalog for the same inputs.
    fn run_serial<P: FnMut(&Progress)>(&self, mut progress: P) -> Result<BulkReport, DataError> {
        use std::sync::atomic::Ordering;
        let order = {
            let cat = Catalog::open(&self.catalog_path)?;
            self.order(&cat)?
        };
        let start = std::time::Instant::now();
        let today = self.today();
        let mut report = BulkReport {
            symbols_total: order.len(),
            ..Default::default()
        };
        {
            let mut cat = Catalog::open(&self.catalog_path)?;
            report.failures = cat.failure_count()?;
            let store = Store::new(&self.store_root);
            store.reconcile(&mut cat)?;
        }
        'tfs: for tf in &self.config.tfs {
            let mut done = 0usize;
            for (id, symbol, instrument) in &order {
                if self.stop.load(Ordering::SeqCst) {
                    report.interrupted = true;
                    break 'tfs;
                }
                let budget_used = {
                    let cat = Catalog::open(&self.catalog_path)?;
                    cat.budget_used(&today)?
                };
                if budget_used >= self.config.budget_cap {
                    report.budget_reached = true;
                    break 'tfs;
                }
                let mut updater = Updater {
                    client: (self.make_client)(),
                    catalog: Catalog::open(&self.catalog_path)?,
                    store: Store::new(&self.store_root),
                    max_retries: self.config.max_retries,
                    margin_seconds: self.config.margin_seconds,
                    reference_symbol_id: self.config.reference_symbol_id.clone(),
                    budget_cap: self.config.budget_cap,
                };
                let before = updater.catalog.budget_used(&today)?;
                let outcome = updater.update_without_reconcile(id, *tf, instrument, self.config.from, self.config.to);
                let after = updater.catalog.budget_used(&today)?;
                report.requests_used += after.saturating_sub(before);
                match outcome {
                    Ok(_) => {
                        done += 1;
                        report.symbols_completed += 1;
                        let cat = Catalog::open(&self.catalog_path)?;
                        cat.clear_failure(id, *tf, self.config.from, self.config.to)?;
                    }
                    Err(e) if e.is_budget() => {
                        report.budget_reached = true;
                        break 'tfs;
                    }
                    Err(e) if e.is_auth() => {
                        return Err(e);
                    }
                    Err(e) => {
                        let cat = Catalog::open(&self.catalog_path)?;
                        let next_retry = chrono::Utc::now().date_naive()
                            + chrono::Days::new(self.config.failure_backoff_days);
                        cat.record_failure(
                            id,
                            *tf,
                            self.config.from,
                            self.config.to,
                            &e.code(),
                            next_retry,
                        )?;
                        report.failures = cat.failure_count()?;
                        eprintln!(
                            "failure {symbol} {} {}..={}: {e}",
                            tf.as_str(),
                            self.config.from,
                            self.config.to
                        );
                    }
                }
                let elapsed = start.elapsed().as_secs_f64().max(1e-3);
                let rate = report.requests_used as f64 / elapsed;
                progress(&Progress {
                    tf: tf.as_str(),
                    done,
                    total: order.len(),
                    requests: report.requests_used,
                    budget_cap: self.config.budget_cap,
                    budget_used: after,
                    rate,
                    failures: report.failures,
                });
            }
        }
        Ok(report)
    }

    /// Across-symbols concurrent download (`config.concurrency > 1`). Within each
    /// timeframe a bounded pool of worker threads (pool size = `concurrency`) each
    /// drives one symbol's [`Updater::update`] to completion on its own SQLite
    /// connection. The pool size is spec 6's in-flight cap; the shared `Limiter`
    /// inside the cloned clients still spaces request *starts* >=250 ms apart
    /// globally, so concurrency only overlaps responses on the wire, never the
    /// 250 ms start spacing.
    ///
    /// Each symbol is a distinct series touched by a single worker, so the overlap
    /// read-modify-write never races; cross-worker safety on the shared `budget`,
    /// `runs` and `trading_calendar` rows comes from WAL + `busy_timeout` and the
    /// IMMEDIATE write transactions. Timeframes are still a barrier (all symbols
    /// for 1d finish before 1m) and each symbol commits whole, so Ctrl-C and
    /// budget stops leave whole symbols behind exactly as the serial path does.
    ///
    /// Relaxed vs. serial: the daily budget is a soft cap here — up to
    /// `concurrency - 1` requests may be issued past it before every worker
    /// observes the stop — and an auth failure aborts only after the in-flight
    /// symbols finish. Callers needing exact accounting use `concurrency = 1`.
    fn run_concurrent<P: FnMut(&Progress) + Send>(
        &self,
        progress: P,
    ) -> Result<BulkReport, DataError> {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Mutex;
        let order = {
            let cat = Catalog::open(&self.catalog_path)?;
            self.order(&cat)?
        };
        let total = order.len();
        let start = std::time::Instant::now();
        let today = self.today();
        let mut initial = BulkReport {
            symbols_total: total,
            ..Default::default()
        };
        {
            let cat = Catalog::open(&self.catalog_path)?;
            initial.failures = cat.failure_count()?;
        }
        let report = Mutex::new(initial);
        let progress = Mutex::new(progress);
        // The first error that the serial path would `?`-propagate (catalog open
        // failure or auth): stored, then returned after the pool drains.
        let fatal: Mutex<Option<DataError>> = Mutex::new(None);
        // Set when any worker hits the budget or a fatal error so the others stop
        // picking up new symbols (in-flight symbols still finish).
        let stop_all = AtomicBool::new(false);

        {
            let mut cat = Catalog::open(&self.catalog_path)?;
            let store = Store::new(&self.store_root);
            store.reconcile(&mut cat)?;
        }

        'tfs: for tf in &self.config.tfs {
            let next = AtomicUsize::new(0);
            let done = AtomicUsize::new(0);
            let workers = self.config.concurrency.max(1).min(total.max(1));
            std::thread::scope(|scope| {
                for _ in 0..workers {
                    scope.spawn(|| {
                        while !self.stop.load(Ordering::SeqCst) && !stop_all.load(Ordering::SeqCst)
                        {
                            let i = next.fetch_add(1, Ordering::SeqCst);
                            if i >= total {
                                break;
                            }
                            let (id, symbol, instrument) = &order[i];
                            // Budget gate before taking on a new symbol (mirrors the
                            // serial per-symbol check).
                            match Catalog::open(&self.catalog_path)
                                .and_then(|c| c.budget_used(&today))
                            {
                                Ok(used) if used >= self.config.budget_cap => {
                                    report.lock().unwrap().budget_reached = true;
                                    stop_all.store(true, Ordering::SeqCst);
                                    break;
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    *fatal.lock().unwrap() = Some(e);
                                    stop_all.store(true, Ordering::SeqCst);
                                    break;
                                }
                            }
                            let mut updater = match Catalog::open(&self.catalog_path) {
                                Ok(catalog) => Updater {
                                    client: (self.make_client)(),
                                    catalog,
                                    store: Store::new(&self.store_root),
                                    max_retries: self.config.max_retries,
                                    margin_seconds: self.config.margin_seconds,
                                    reference_symbol_id: self.config.reference_symbol_id.clone(),
                                    budget_cap: self.config.budget_cap,
                                },
                                Err(e) => {
                                    *fatal.lock().unwrap() = Some(e);
                                    stop_all.store(true, Ordering::SeqCst);
                                    break;
                                }
                            };
                            let before = updater.catalog.budget_used(&today).unwrap_or(0);
                            let outcome = updater.update_without_reconcile(
                                id,
                                *tf,
                                instrument,
                                self.config.from,
                                self.config.to,
                            );
                            let after = updater
                                .catalog
                                .budget_used(&today)
                                .unwrap_or(before)
                                .max(before);
                            report.lock().unwrap().requests_used += after.saturating_sub(before);
                            match outcome {
                                Ok(_) => {
                                    report.lock().unwrap().symbols_completed += 1;
                                    if let Ok(cat) = Catalog::open(&self.catalog_path) {
                                        let _ = cat.clear_failure(
                                            id,
                                            *tf,
                                            self.config.from,
                                            self.config.to,
                                        );
                                    }
                                }
                                Err(e) if e.is_budget() => {
                                    report.lock().unwrap().budget_reached = true;
                                    stop_all.store(true, Ordering::SeqCst);
                                }
                                Err(e) if e.is_auth() => {
                                    *fatal.lock().unwrap() = Some(e);
                                    stop_all.store(true, Ordering::SeqCst);
                                }
                                Err(e) => {
                                    if let Ok(cat) = Catalog::open(&self.catalog_path) {
                                        let next_retry = chrono::Utc::now().date_naive()
                                            + chrono::Days::new(self.config.failure_backoff_days);
                                        let _ = cat.record_failure(
                                            id,
                                            *tf,
                                            self.config.from,
                                            self.config.to,
                                            &e.code(),
                                            next_retry,
                                        );
                                        if let Ok(fc) = cat.failure_count() {
                                            report.lock().unwrap().failures = fc;
                                        }
                                    }
                                    eprintln!(
                                        "failure {symbol} {} {}..={}: {e}",
                                        tf.as_str(),
                                        self.config.from,
                                        self.config.to
                                    );
                                }
                            }
                            let d = done.fetch_add(1, Ordering::SeqCst) + 1;
                            let (requests_used, failures) = {
                                let r = report.lock().unwrap();
                                (r.requests_used, r.failures)
                            };
                            let elapsed = start.elapsed().as_secs_f64().max(1e-3);
                            let rate = requests_used as f64 / elapsed;
                            let snapshot = Progress {
                                tf: tf.as_str(),
                                done: d,
                                total,
                                requests: requests_used,
                                budget_cap: self.config.budget_cap,
                                budget_used: after,
                                rate,
                                failures,
                            };
                            let mut p = progress.lock().unwrap();
                            (*p)(&snapshot);
                        }
                    });
                }
            });
            if self.stop.load(Ordering::SeqCst) {
                report.lock().unwrap().interrupted = true;
                break 'tfs;
            }
            if fatal.lock().unwrap().is_some() {
                break 'tfs;
            }
            if report.lock().unwrap().budget_reached {
                break 'tfs;
            }
        }
        if let Some(e) = fatal.lock().unwrap().take() {
            return Err(e);
        }
        Ok(report.into_inner().unwrap())
    }
}

/// A snapshot handed to the progress callback once per symbol.
#[derive(Clone, Debug)]
pub struct Progress<'a> {
    pub tf: &'a str,
    pub done: usize,
    pub total: usize,
    pub requests: u64,
    pub budget_cap: u64,
    pub budget_used: u64,
    pub rate: f64,
    pub failures: u64,
}

impl Progress<'_> {
    /// The single updating status line from spec 9, e.g.
    /// `1m  842/2365 symbols | requests 17,904/90,000 today | 3.9 req/s | ETA 2h 41m | failures 3`.
    pub fn line(&self) -> String {
        let remaining = self.total.saturating_sub(self.done);
        let eta = if self.rate > 0.0 {
            // Rough: assume the remaining symbols issue a similar number of
            // requests per symbol as issued so far.
            let per_symbol = if self.done > 0 {
                self.requests as f64 / self.done as f64
            } else {
                0.0
            };
            let secs = (remaining as f64 * per_symbol / self.rate) as i64;
            format_eta(secs)
        } else {
            "--".into()
        };
        format!(
            "{tf}  {done}/{total} symbols | requests {req}/{cap} today | {rate:.1} req/s | ETA {eta} | failures {fail}",
            tf = self.tf,
            done = self.done,
            total = self.total,
            req = group_thousands(self.budget_used),
            cap = group_thousands(self.budget_cap),
            rate = self.rate,
            eta = eta,
            fail = self.failures,
        )
    }
}

pub(crate) fn format_eta(secs: i64) -> String {
    if secs <= 0 {
        return "0m".into();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}

pub(crate) fn group_thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::new();
    for (i, c) in bytes.iter().enumerate() {
        if i > 0 && (bytes.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*c as char);
    }
    out
}
