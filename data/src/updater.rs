use crate::catalog::Catalog;
use crate::error::DataError;
use crate::history::HistoryApi;
use crate::session::{
    check_overlap, chunk_dates, chunk_dates_limited, complete_intraday, local_midnight,
    missing_date_ranges, session_regime_changes, session_shapes,
};
use crate::store::{read_bars, Store};
use crate::types::{Bar, Quality, Tf};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use chrono_tz::Asia::Kolkata;
use rusqlite::params;
use std::collections::HashMap;

pub const DEFAULT_BUDGET_CAP: u64 = 90_000;

pub(crate) fn instrument_interval(tf: Tf) -> &'static str {
    match tf {
        Tf::M1 => "1",
        Tf::M5 => "5",
        Tf::M15 => "15",
        Tf::M25 => "25",
        Tf::M60 => "60",
        Tf::D1 => "day",
    }
}

pub struct Updater {
    pub client: Box<dyn HistoryApi>,
    pub catalog: Catalog,
    pub store: Store,
    pub max_retries: u32,
    pub margin_seconds: i64,
    pub reference_symbol_id: Option<String>,
    /// Soft cap on requests per IST day. When `budget_used >= budget_cap` the
    /// updater stops with `DataError::BudgetExhausted`. Defaults to 90,000.
    pub budget_cap: u64,
}

impl Updater {
    fn budget_guard(&self, today: &str) -> Result<(), DataError> {
        let used = self.catalog.budget_used(today)?;
        if used >= self.budget_cap {
            return Err(DataError::BudgetExhausted {
                used,
                cap: self.budget_cap,
            });
        }
        Ok(())
    }

    pub fn update(
        &mut self,
        security_id: &str,
        tf: Tf,
        instrument: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<bool, DataError> {
        self.update_internal(security_id, tf, instrument, from, to, true)
    }

    pub fn update_without_reconcile(
        &mut self,
        security_id: &str,
        tf: Tf,
        instrument: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<bool, DataError> {
        self.update_internal(security_id, tf, instrument, from, to, false)
    }

    fn update_internal(
        &mut self,
        security_id: &str,
        tf: Tf,
        instrument: &str,
        from: NaiveDate,
        to: NaiveDate,
        do_reconcile: bool,
    ) -> Result<bool, DataError> {
        let today = chrono::Utc::now().date_naive().to_string();
        self.budget_guard(&today)?;
        let run = self.catalog.runs_start()?;
        let mut requests = 0;
        let result = (|| {
            if do_reconcile {
                self.store.reconcile(&mut self.catalog)?;
            }
            let gaps = missing_date_ranges(&self.catalog, security_id, tf, from, to)?;
            let chunks = if let (Some((first, _)), Some((_, last))) = (gaps.first(), gaps.last()) {
                if tf == Tf::D1 {
                    vec![(*first, *last)]
                } else {
                    chunk_dates(*first, *last)
                }
            } else {
                Vec::new()
            };
            for (a, b) in chunks.iter().copied() {
                let pending = missing_date_ranges(&self.catalog, security_id, tf, a, b)?;
                if pending.is_empty() {
                    continue;
                }
                for (request_from, request_to) in pending {
                    self.budget_guard(&today)?;
                    eprintln!(
                        "request {} {} {}..={}",
                        security_id,
                        tf.as_str(),
                        request_from,
                        request_to
                    );
                    let fetch = if tf == Tf::D1 {
                        self.client
                            .daily(security_id, instrument, request_from, request_to)
                    } else {
                        self.client.intraday(
                            security_id,
                            instrument,
                            instrument_interval(tf),
                            request_from,
                            request_to,
                        )
                    };
                    requests += 1;
                    self.catalog.budget_add(&today, 1)?;
                    let candles = match fetch {
                        Ok(rows) => rows,
                        Err(error)
                            if tf == Tf::D1
                                && (request_to - request_from).num_days() + 1 > 1825 =>
                        {
                            eprintln!("daily request failed for {request_from}..={request_to}; retrying in 5-year fallback chunks: {error}");
                            let mut rows = Vec::new();
                            for (fallback_from, fallback_to) in
                                chunk_dates_limited(request_from, request_to, 1825)
                            {
                                self.budget_guard(&today)?;
                                let part = self.client.daily(
                                    security_id,
                                    instrument,
                                    fallback_from,
                                    fallback_to,
                                );
                                requests += 1;
                                self.catalog.budget_add(&today, 1)?;
                                rows.extend(part?);
                            }
                            rows
                        }
                        Err(error) => return Err(error),
                    };
                    let bars: Vec<Bar> = candles;
                    if bars.is_empty() {
                        let from_ts = local_midnight(a);
                        let to_ts = local_midnight(b + chrono::Days::new(1)) - 1;
                        if self.reference_symbol_id.as_deref() == Some(security_id) {
                            for n in 0..=((b - a).num_days().max(0) as u64) {
                                let d = a + chrono::Days::new(n);
                                if matches!(
                                    d.weekday(),
                                    chrono::Weekday::Sat | chrono::Weekday::Sun
                                ) {
                                    self.catalog.record_calendar(d, false, "weekend")?;
                                } else {
                                    self.catalog.record_calendar(d, false, "reference-empty")?;
                                }
                            }
                        }
                        let _ = (from_ts, to_ts);
                        continue;
                    }
                    let now = chrono::Utc::now().timestamp();
                    let saved: Vec<Bar> = if tf == Tf::D1 {
                        let current_local = DateTime::<Utc>::from_timestamp(now, 0)
                            .unwrap_or(DateTime::UNIX_EPOCH)
                            .with_timezone(&Kolkata);
                        bars.into_iter()
                            .filter(|b| {
                                let session_date =
                                    DateTime::<Utc>::from_timestamp(b.ts - 33_300, 0)
                                        .unwrap_or(DateTime::UNIX_EPOCH)
                                        .with_timezone(&Kolkata)
                                        .date_naive();
                                let ready_at = session_date
                                    .and_hms_opt(16, 0, 0)
                                    .unwrap()
                                    .and_local_timezone(Kolkata)
                                    .single()
                                    .unwrap()
                                    .timestamp();
                                now >= ready_at || session_date < current_local.date_naive()
                            })
                            .collect()
                    } else {
                        complete_intraday(bars, now, tf, &HashMap::new(), self.margin_seconds)
                    };
                    if saved.is_empty() {
                        continue;
                    }
                    let existing_all: Vec<Bar> = if let Ok(sid) = self.catalog.series(security_id, tf) {
                        let mut q = self
                            .catalog
                            .conn
                            .prepare("SELECT path FROM partitions WHERE series_id=?1")?;
                        let paths = q
                            .query_map([sid], |r| r.get::<_, String>(0))?
                            .collect::<Result<Vec<_>, _>>()?;
                        let mut all = Vec::new();
                        for rel in paths {
                            all.extend(read_bars(&self.store.root.join(rel))?)
                        }
                        all
                    } else {
                        Vec::new()
                    };
                    let (kind, diffs) = check_overlap(&existing_all, &saved);
                    if kind == "corporate_action" {
                        let sid = self.catalog.series(security_id, tf)?;
                        self.catalog
                            .conn
                            .execute("UPDATE series SET needs_refresh=1 WHERE id=?1", [sid])?;
                        self.store.remove_series_files(security_id, tf)?;
                        self.catalog
                            .conn
                            .execute("DELETE FROM partitions WHERE series_id=?1", [sid])?;
                        self.catalog
                            .conn
                            .execute("DELETE FROM coverage WHERE series_id=?1", [sid])?;
                        self.catalog
                            .conn
                            .execute("DELETE FROM sessions WHERE series_id=?1", [sid])?;
                        self.catalog.conn.execute(
                            "UPDATE series SET first_ts=NULL,last_ts=NULL,rows=0 WHERE id=?1",
                            [sid],
                        )?;
                        for (full_from, full_to) in chunks.clone() {
                            self.budget_guard(&today)?;
                            let full = if tf == Tf::D1 {
                                self.client
                                    .daily(security_id, instrument, full_from, full_to)?
                            } else {
                                self.client.intraday(
                                    security_id,
                                    instrument,
                                    instrument_interval(tf),
                                    full_from,
                                    full_to,
                                )?
                            };
                            requests += 1;
                            self.catalog.budget_add(&today, 1)?;
                            let complete = if tf == Tf::D1 {
                                full
                            } else {
                                complete_intraday(
                                    full,
                                    chrono::Utc::now().timestamp(),
                                    tf,
                                    &HashMap::new(),
                                    self.margin_seconds,
                                )
                            };
                            if complete.is_empty() {
                                continue;
                            }
                            let parts = self.store.write_partitions(security_id, tf, &complete)?;
                            for p in parts {
                                self.catalog.record_partition(sid, &self.store, &p)?;
                            }
                        }
                        self.catalog
                            .conn
                            .execute("UPDATE series SET needs_refresh=0 WHERE id=?1", [sid])?;
                        return Ok(());
                    }
                    if kind == "revised" && !diffs.is_empty() {
                        self.catalog.mark_quality(
                            security_id,
                            tf,
                            &Quality {
                                kind: "revised".into(),
                                ts: diffs.first().copied(),
                                detail: format!("{} revised bars in overlap", diffs.len()),
                            },
                        )?;
                    }
                    let partitions = self.store.write_partitions(security_id, tf, &saved)?;
                    let sid = self.catalog.ensure_series(security_id, tf)?;
                    let tx = self
                        .catalog
                        .conn
                        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                    for part in &partitions {
                        let rel = part
                            .path
                            .strip_prefix(&self.store.root)
                            .unwrap_or(&part.path)
                            .to_string_lossy()
                            .to_string();
                        tx.execute("INSERT INTO partitions VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(series_id,path) DO UPDATE SET first_ts=excluded.first_ts,last_ts=excluded.last_ts,rows=excluded.rows,checksum=excluded.checksum",params![sid,rel,part.first_ts,part.last_ts,part.rows,part.checksum])?;
                    }
                    let first = saved.iter().map(|x| x.ts).min().unwrap();
                    let last = saved.iter().map(|x| x.ts).max().unwrap();
                    let shapes = session_shapes(&saved);
                    for day in session_regime_changes(&shapes) {
                        tx.execute("INSERT INTO quality(series_id,kind,ts,detail) SELECT ?1,'session_regime_change',?2,'derived session end mode changed' WHERE NOT EXISTS (SELECT 1 FROM quality WHERE series_id=?1 AND kind='session_regime_change' AND ts=?2)",params![sid,local_midnight(day)])?;
                    }
                    for (d, (f, l, n)) in shapes {
                        tx.execute(
                            "INSERT INTO coverage VALUES(?1,?2,?3,'data')",
                            params![
                                sid,
                                local_midnight(d),
                                local_midnight(d + chrono::Days::new(1)) - 1
                            ],
                        )?;
                        if self.reference_symbol_id.as_deref() == Some(security_id) {
                            tx.execute("INSERT INTO trading_calendar VALUES(?1,1,'reference-bars') ON CONFLICT(date) DO UPDATE SET is_trading=1,source='reference-bars'",[d.to_string()])?;
                            tx.execute(
                                "DELETE FROM trading_calendar WHERE date=?1 AND is_trading=0",
                                [d.to_string()],
                            )?;
                            tx.execute("DELETE FROM coverage WHERE series_id=?1 AND kind='confirmed_empty' AND start_ts>=?2 AND start_ts<?3",params![sid,local_midnight(d),local_midnight(d+chrono::Days::new(1))])?;
                            tx.execute(
                                "DELETE FROM sessions WHERE series_id=?1 AND date=?2 AND bars=0",
                                params![sid, d.to_string()],
                            )?;
                        }
                        tx.execute("INSERT INTO sessions VALUES(?1,?2,?3,?4,?5) ON CONFLICT(series_id,date) DO UPDATE SET first_ts=excluded.first_ts,last_ts=excluded.last_ts,bars=excluded.bars",params![sid,d.to_string(),f,l,n])?;
                    }
                    tx.execute("UPDATE series SET first_ts=CASE WHEN first_ts IS NULL THEN ?2 ELSE MIN(first_ts,?2) END,last_ts=CASE WHEN last_ts IS NULL THEN ?3 ELSE MAX(last_ts,?3) END,rows=(SELECT COALESCE(SUM(rows),0) FROM partitions WHERE series_id=?1) WHERE id=?1",params![sid,first,last])?;
                    tx.commit()?;
                    if self.reference_symbol_id.as_deref() == Some(security_id) {
                        for day in self.catalog.missing_dates(security_id, tf, a, b)? {
                            self.catalog
                                .record_calendar(day, false, "reference-empty")?;
                        }
                    }
                    if !security_id.is_empty() {
                        for day in self.catalog.missing_dates(security_id, tf, a, b)? {
                            let is_open:bool=self.catalog.conn.query_row("SELECT COALESCE((SELECT is_trading=1 FROM trading_calendar WHERE date=?1),1)",[day.to_string()],|r|r.get(0))?;
                            if !is_open {
                                self.catalog.mark_session_empty(security_id, tf, day)?;
                                continue;
                            }
                            let mut recovered = false;
                            for _ in 0..self.max_retries {
                                self.budget_guard(&today)?;
                                let retry = if tf == Tf::D1 {
                                    self.client.daily(security_id, instrument, day, day)?
                                } else {
                                    self.client.intraday(
                                        security_id,
                                        instrument,
                                        instrument_interval(tf),
                                        day,
                                        day,
                                    )?
                                };
                                requests += 1;
                                self.catalog.budget_add(&today, 1)?;
                                if !retry.is_empty() {
                                    let retry_bars: Vec<Bar> = retry;
                                    let completed = if tf == Tf::D1 {
                                        retry_bars
                                    } else {
                                        complete_intraday(
                                            retry_bars,
                                            chrono::Utc::now().timestamp(),
                                            tf,
                                            &HashMap::new(),
                                            self.margin_seconds,
                                        )
                                    };
                                    if !completed.is_empty() {
                                        let pieces = self.store.write_partitions(
                                            security_id,
                                            tf,
                                            &completed,
                                        )?;
                                        let sid = self.catalog.ensure_series(security_id, tf)?;
                                        for part in pieces {
                                            self.catalog.record_partition(
                                                sid,
                                                &self.store,
                                                &part,
                                            )?;
                                        }
                                        recovered = true;
                                        break;
                                    }
                                }
                            }
                            if !recovered {
                                if self.reference_symbol_id.as_deref() == Some(security_id) {
                                    self.catalog
                                        .record_calendar(day, false, "reference-empty")?;
                                }
                                self.catalog.mark_session_empty(security_id, tf, day)?;
                                self.catalog.mark_quality(security_id,tf,&Quality{kind:"missing_symbol_session".into(),ts:Some(local_midnight(day)),detail:"no bars after bounded per-symbol retry; marked confirmed_empty".into()})?;
                            }
                        }
                    }
                }
            }
            Ok::<(), DataError>(())
        })();
        self.catalog.runs_finish(
            run,
            requests,
            if result.is_ok() { "complete" } else { "failed" },
        )?;
        let changed = requests > 0;
        result?;
        Ok(changed)
    }
}

/// Number of Dhan requests the planner will issue for `id`/`tf` over
/// `from..=to`, given current catalog coverage. Mirrors the chunking in
/// [`Updater::update`]: a daily series is a single request (5-year fallback
/// chunks only happen after a data error), an intraday series is one request
/// per 89-calendar-day chunk over the merged missing range. Returns zero when
/// the range is already covered, which is what makes `--dry-run` after an
/// interrupted run report only the remaining work (spec 7).
pub fn plan_requests(
    cat: &Catalog,
    id: &str,
    tf: Tf,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<usize, DataError> {
    let gaps = missing_date_ranges(cat, id, tf, from, to)?;
    Ok(match (gaps.first(), gaps.last()) {
        (Some((first, _)), Some((_, last))) => {
            if tf == Tf::D1 {
                1
            } else {
                chunk_dates(*first, *last).len()
            }
        }
        _ => 0,
    })
}
