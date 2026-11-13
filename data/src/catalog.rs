use crate::error::DataError;
use crate::session::{local_midnight, ts_to_ist_date};
use crate::store::Store;
use crate::types::{
    Coverage, CoverageRange, PartitionMeta, Quality, QualityRow, SeriesStatus, Tf,
};
use chrono::{DateTime, Datelike, NaiveDate, Utc};
use chrono_tz::Asia::Kolkata;
use dhan_api::Scrip;
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

pub struct Catalog {
    pub(crate) conn: Connection,
}

impl Catalog {
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, DataError> {
        let p = path.as_ref();
        let conn = match Connection::open_with_flags(
            p,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            Ok(c) => c,
            Err(_) => Connection::open(p)?,
        };
        let _ = conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=30000;");
        Ok(Self { conn })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, DataError> {
        if let Some(p) = path.as_ref().parent() {
            fs::create_dir_all(p)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=30000; CREATE TABLE IF NOT EXISTS instruments(security_id TEXT PRIMARY KEY,symbol TEXT NOT NULL UNIQUE,segment TEXT NOT NULL,lot_size INTEGER NOT NULL,tick_size_paise INTEGER NOT NULL,instrument TEXT NOT NULL DEFAULT 'EQUITY',is_fno INTEGER NOT NULL DEFAULT 0,refreshed_at INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS series(id INTEGER PRIMARY KEY,security_id TEXT NOT NULL,tf TEXT NOT NULL,first_ts INTEGER,last_ts INTEGER,rows INTEGER NOT NULL DEFAULT 0,needs_refresh INTEGER NOT NULL DEFAULT 0,UNIQUE(security_id,tf)); CREATE TABLE IF NOT EXISTS partitions(series_id INTEGER NOT NULL,path TEXT NOT NULL,first_ts INTEGER NOT NULL,last_ts INTEGER NOT NULL,rows INTEGER NOT NULL,checksum TEXT NOT NULL,PRIMARY KEY(series_id,path)); CREATE TABLE IF NOT EXISTS coverage(series_id INTEGER NOT NULL,start_ts INTEGER NOT NULL,end_ts INTEGER NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('data','confirmed_empty'))); CREATE TABLE IF NOT EXISTS sessions(series_id INTEGER NOT NULL,date TEXT NOT NULL,first_ts INTEGER NOT NULL,last_ts INTEGER NOT NULL,bars INTEGER NOT NULL,PRIMARY KEY(series_id,date)); CREATE TABLE IF NOT EXISTS quality(id INTEGER PRIMARY KEY,series_id INTEGER,kind TEXT NOT NULL,ts INTEGER,detail TEXT NOT NULL); CREATE TABLE IF NOT EXISTS runs(id INTEGER PRIMARY KEY,started INTEGER NOT NULL,finished INTEGER,requests INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL); CREATE TABLE IF NOT EXISTS budget(day TEXT PRIMARY KEY,requests INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS trading_calendar(date TEXT PRIMARY KEY,is_trading INTEGER NOT NULL,source TEXT NOT NULL); CREATE TABLE IF NOT EXISTS failures(security_id INTEGER,tf TEXT,from_date TEXT,to_date TEXT,error_code TEXT,attempts INTEGER,next_retry TEXT,PRIMARY KEY(security_id,tf,from_date,to_date)); CREATE INDEX IF NOT EXISTS idx_partitions_range ON partitions(series_id, first_ts, last_ts); CREATE INDEX IF NOT EXISTS idx_coverage_range ON coverage(series_id, start_ts, end_ts); CREATE INDEX IF NOT EXISTS idx_quality_series ON quality(series_id, kind);")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 2 {
            return Err(DataError::Invalid(format!(
                "catalog schema version {version} is newer than supported version 2"
            )));
        }
        let (has_instrument, has_is_fno): (bool, bool) = {
            let mut q = conn.prepare("PRAGMA table_info(instruments)")?;
            let names = q
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            (
                names.iter().any(|x| x == "instrument"),
                names.iter().any(|x| x == "is_fno"),
            )
        };
        if !has_instrument {
            conn.execute(
                "ALTER TABLE instruments ADD COLUMN instrument TEXT NOT NULL DEFAULT 'EQUITY'",
                [],
            )?;
        }
        if !has_is_fno {
            conn.execute(
                "ALTER TABLE instruments ADD COLUMN is_fno INTEGER NOT NULL DEFAULT 0",
                [],
            )?;
        }
        conn.execute_batch("PRAGMA user_version=2;")?;
        Ok(Self { conn })
    }

    pub fn refresh_instruments(&mut self, rows: &[Scrip], at: i64) -> Result<(), DataError> {
        let tx = self.conn.transaction()?;
        for x in rows {
            tx.execute(
                "INSERT INTO instruments(security_id,symbol,segment,lot_size,tick_size_paise,instrument,is_fno,refreshed_at) \
                 VALUES(?1,?2,'NSE_EQ',?3,?4,?5,?6,?7) \
                 ON CONFLICT(security_id) DO UPDATE SET \
                 symbol=excluded.symbol,lot_size=excluded.lot_size,tick_size_paise=excluded.tick_size_paise,\
                 instrument=excluded.instrument,is_fno=excluded.is_fno,refreshed_at=excluded.refreshed_at",
                params![x.security_id, x.symbol, x.lot_size, x.tick_size_paise, x.instrument, if x.is_fno { 1 } else { 0 }, at],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn is_fno(&self, id: &str) -> Result<bool, DataError> {
        Ok(self.conn.query_row(
            "SELECT is_fno FROM instruments WHERE security_id=?1",
            [id],
            |r| Ok(r.get::<_, i64>(0)? != 0),
        )?)
    }

    pub fn instrument_counts(&self) -> Result<(usize, usize), DataError> {
        let total: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM instruments", [], |r| r.get(0))?;
        let fno: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM instruments WHERE is_fno != 0",
            [],
            |r| r.get(0),
        )?;
        Ok((total, fno))
    }

    pub fn instrument(&self, id: &str) -> Result<String, DataError> {
        Ok(self.conn.query_row(
            "SELECT instrument FROM instruments WHERE security_id=?1",
            [id],
            |r| r.get(0),
        )?)
    }

    pub fn resolve(&self, s: &str) -> Result<(String, String), DataError> {
        self.conn
            .query_row(
                "SELECT security_id,symbol FROM instruments WHERE symbol=?1",
                [s.trim().to_ascii_uppercase()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|_| DataError::Invalid(format!("unknown symbol {}", s)))
    }

    pub fn series_for_import(&self, id: &str, tf: Tf) -> Result<i64, DataError> {
        self.ensure_series(id, tf)
    }

    pub fn record_partition(
        &self,
        series: i64,
        store: &Store,
        part: &PartitionMeta,
    ) -> Result<(), DataError> {
        let rel = part
            .path
            .strip_prefix(&store.root)
            .unwrap_or(&part.path)
            .to_string_lossy()
            .to_string();
        let (sum, first, last, rows) = (&part.checksum, part.first_ts, part.last_ts, part.rows);
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("INSERT INTO partitions VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(series_id,path) DO UPDATE SET first_ts=excluded.first_ts,last_ts=excluded.last_ts,rows=excluded.rows,checksum=excluded.checksum",params![series,rel,first,last,rows,sum])?;
        let d1 = DateTime::<Utc>::from_timestamp(first, 0)
            .unwrap_or(DateTime::UNIX_EPOCH)
            .with_timezone(&Kolkata)
            .date_naive();
        let d2 = DateTime::<Utc>::from_timestamp(last, 0)
            .unwrap_or(DateTime::UNIX_EPOCH)
            .with_timezone(&Kolkata)
            .date_naive();
        tx.execute(
            "INSERT INTO coverage VALUES(?1,?2,?3,'data')",
            params![
                series,
                local_midnight(d1),
                local_midnight(d2 + chrono::Days::new(1)) - 1
            ],
        )?;
        tx.execute("UPDATE series SET first_ts=CASE WHEN first_ts IS NULL THEN ?2 ELSE MIN(first_ts,?2) END,last_ts=CASE WHEN last_ts IS NULL THEN ?3 ELSE MAX(last_ts,?3) END,rows=(SELECT COALESCE(SUM(rows),0) FROM partitions WHERE series_id=?1) WHERE id=?1",params![series,first,last])?;
        tx.commit()?;
        Ok(())
    }

    pub fn quality(&self, kind: &str) -> Result<u64, DataError> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM quality WHERE kind=?1", [kind], |r| {
                r.get(0)
            })?)
    }

    pub fn series(&self, id: &str, tf: Tf) -> Result<i64, DataError> {
        self.conn
            .query_row(
                "SELECT id FROM series WHERE security_id=?1 AND tf=?2",
                params![id, tf.as_str()],
                |r| r.get(0),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => DataError::Uncovered(id.to_string()),
                other => DataError::Db(other),
            })
    }

    pub fn ensure_series(&self, id: &str, tf: Tf) -> Result<i64, DataError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO series(security_id,tf) VALUES(?1,?2)",
            params![id, tf.as_str()],
        )?;
        self.series(id, tf)
    }

    pub fn record_session(
        &self,
        sid: i64,
        date: NaiveDate,
        first_ts: i64,
        last_ts: i64,
        bars: u64,
    ) -> Result<(), DataError> {
        self.conn.execute(
            "INSERT INTO sessions VALUES(?1,?2,?3,?4,?5) ON CONFLICT(series_id,date) DO UPDATE SET first_ts=excluded.first_ts,last_ts=excluded.last_ts,bars=excluded.bars",
            params![sid, date.to_string(), first_ts, last_ts, bars],
        )?;
        Ok(())
    }

    pub fn instrument_meta(&self, security_id: &str) -> Result<(u64, i64, bool), DataError> {
        self.conn.query_row(
            "SELECT lot_size, tick_size_paise, is_fno FROM instruments WHERE security_id=?1",
            [security_id],
            |r| {
                let lot: u64 = r.get(0)?;
                let tick: i64 = r.get(1)?;
                let fno: i64 = r.get(2)?;
                Ok((lot.max(1), tick.max(1), fno != 0))
            },
        ).map_err(DataError::Db)
    }

    pub fn is_holiday(&self, date: NaiveDate) -> Result<bool, DataError> {
        let not_trading: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM trading_calendar WHERE date=?1 AND is_trading=0)",
            [date.to_string()],
            |r| r.get(0),
        )?;
        Ok(not_trading)
    }

    pub fn first_covered_date(&self, sid: i64) -> Result<Option<NaiveDate>, DataError> {
        let date_str: Option<String> = self
            .conn
            .query_row(
                "SELECT MIN(date) FROM sessions WHERE series_id=?1",
                [sid],
                |r| r.get(0),
            )
            .ok();
        if let Some(s) = date_str {
            if let Ok(d) = NaiveDate::parse_from_str(&s, "%Y-%m-%d") {
                return Ok(Some(d));
            }
        }
        let first_ts: Option<i64> = self
            .conn
            .query_row(
                "SELECT first_ts FROM series WHERE id=?1",
                [sid],
                |r| r.get(0),
            )
            .ok()
            .flatten();
        if let Some(ts) = first_ts {
            return Ok(Some(ts_to_ist_date(ts)));
        }
        Ok(None)
    }

    pub fn bars_before_ts(&self, sid: i64, from_date: NaiveDate, cutoff_ts: i64) -> Result<usize, DataError> {
        let session_count: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(bars), 0) FROM sessions WHERE series_id=?1 AND date>=?2 AND first_ts < ?3",
            params![sid, from_date.to_string(), cutoff_ts],
            |r| r.get(0),
        )?;
        if session_count > 0 {
            return Ok(session_count as usize);
        }
        let partition_count: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(rows), 0) FROM partitions WHERE series_id=?1 AND first_ts < ?2",
            params![sid, cutoff_ts],
            |r| r.get(0),
        )?;
        Ok(partition_count as usize)
    }

    pub fn has_coverage(&self, id: &str, tf: Tf, from: i64, to: i64) -> Result<bool, DataError> {
        let first = ts_to_ist_date(from);
        let last = ts_to_ist_date(to);
        if first > last {
            return Ok(false);
        }
        let missing = self.missing_dates(id, tf, first, last)?;
        Ok(missing.is_empty())
    }

    pub fn record_coverage(
        &self,
        id: &str,
        tf: Tf,
        start: i64,
        end: i64,
        kind: &str,
    ) -> Result<(), DataError> {
        if start > end || !(kind == "data" || kind == "confirmed_empty") {
            return Err(DataError::Invalid("invalid coverage interval".into()));
        }
        let sid = self.ensure_series(id, tf)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO coverage VALUES(?1,?2,?3,?4)",
            params![sid, start, end, kind],
        )?;
        Ok(())
    }

    pub fn covered_dates(
        &self,
        id: &str,
        tf: Tf,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<NaiveDate>, DataError> {
        let sid = self.series(id, tf).ok();
        let xs: Vec<String> = if let Some(sid) = sid {
            let mut q = self.conn.prepare(
                "SELECT date FROM sessions WHERE series_id=?1 AND date>=?2 AND date<=?3 UNION SELECT date FROM trading_calendar WHERE is_trading=0 AND date>=?2 AND date<=?3 ORDER BY date",
            )?;
            let rows = q.query_map(params![sid, from.to_string(), to.to_string()], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
            rows
        } else {
            let mut q = self.conn.prepare(
                "SELECT date FROM trading_calendar WHERE is_trading=0 AND date>=?1 AND date<=?2 ORDER BY date",
            )?;
            let rows = q.query_map(params![from.to_string(), to.to_string()], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
            rows
        };
        xs.into_iter()
            .map(|x| {
                let text = if x.contains('-') && x.len() > 10 {
                    let ts: i64 = x.parse().unwrap_or_default();
                    DateTime::<Utc>::from_timestamp(ts, 0)
                        .unwrap_or(DateTime::UNIX_EPOCH)
                        .with_timezone(&Kolkata)
                        .date_naive()
                        .to_string()
                } else {
                    x
                };
                NaiveDate::parse_from_str(&text, "%Y-%m-%d")
                    .map_err(|e| DataError::Invalid(e.to_string()))
            })
            .collect()
    }

    pub fn record_calendar(
        &self,
        date: NaiveDate,
        is_trading: bool,
        source: &str,
    ) -> Result<(), DataError> {
        self.conn.execute("INSERT INTO trading_calendar VALUES(?1,?2,?3) ON CONFLICT(date) DO UPDATE SET is_trading=excluded.is_trading,source=excluded.source",params![date.to_string(),is_trading,source])?;
        Ok(())
    }

    pub fn mark_session_empty(&self, id: &str, tf: Tf, date: NaiveDate) -> Result<(), DataError> {
        let sid = self.ensure_series(id, tf)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO coverage VALUES(?1,?2,?3,'confirmed_empty')",
            params![
                sid,
                local_midnight(date),
                local_midnight(date + chrono::Days::new(1)) - 1
            ],
        )?;
        self.conn.execute("INSERT OR IGNORE INTO sessions(series_id,date,first_ts,last_ts,bars) VALUES(?1,?2,?3,?3,0)", params![sid,date.to_string(),local_midnight(date)])?;
        Ok(())
    }

    pub fn missing_dates(
        &self,
        id: &str,
        tf: Tf,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<NaiveDate>, DataError> {
        if from > to {
            return Ok(Vec::new());
        }
        let covered = self
            .covered_dates(id, tf, from, to)?
            .into_iter()
            .collect::<std::collections::HashSet<_>>();
        Ok((0..=((to - from).num_days().max(0) as u64))
            .map(|n| from + chrono::Days::new(n))
            .filter(|d| {
                matches!(
                    d.weekday(),
                    chrono::Weekday::Mon
                        | chrono::Weekday::Tue
                        | chrono::Weekday::Wed
                        | chrono::Weekday::Thu
                        | chrono::Weekday::Fri
                ) && !covered.contains(d)
            })
            .collect())
    }

    pub fn coverage(&self) -> Result<Vec<Coverage>, DataError> {
        let mut q = self
            .conn
            .prepare("SELECT start_ts,end_ts,kind FROM coverage ORDER BY series_id,start_ts")?;
        let rows = q.query_map([], |r| {
            Ok(Coverage {
                start_ts: r.get(0)?,
                end_ts: r.get(1)?,
                kind: r.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn coverage_details(&self) -> Result<Vec<(String, String, u64)>, DataError> {
        let mut q=self.conn.prepare("SELECT i.symbol,s.tf,COALESCE(SUM(p.rows),0) FROM series s JOIN instruments i ON i.security_id=s.security_id LEFT JOIN partitions p ON p.series_id=s.id GROUP BY s.id ORDER BY i.symbol,s.tf")?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn status_rows(&self) -> Result<Vec<SeriesStatus>, DataError> {
        let mut q = self.conn.prepare("SELECT s.id,i.symbol,s.tf,COALESCE((SELECT SUM(p.rows) FROM partitions p WHERE p.series_id=s.id),0) FROM series s JOIN instruments i ON i.security_id=s.security_id ORDER BY i.symbol,s.tf")?;
        let series = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, u64>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(series.len());
        for (id, symbol, timeframe, bars) in series {
            let mut stmt = self.conn.prepare(
                "SELECT start_ts,end_ts,kind FROM coverage WHERE series_id=?1 ORDER BY start_ts",
            )?;
            let rows = stmt
                .query_map([id], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            let mut coverage = Vec::<CoverageRange>::new();
            for (a, b, kind) in rows {
                let from = DateTime::<Utc>::from_timestamp(a, 0)
                    .unwrap_or(DateTime::UNIX_EPOCH)
                    .with_timezone(&Kolkata)
                    .date_naive();
                let to = DateTime::<Utc>::from_timestamp(b, 0)
                    .unwrap_or(DateTime::UNIX_EPOCH)
                    .with_timezone(&Kolkata)
                    .date_naive();
                let joins_previous = if let Some(last) = coverage.last() {
                    from <= last.to + chrono::Days::new(1)
                        || self.only_nontrading_gap(last.to, from)?
                } else {
                    false
                };
                if joins_previous {
                    if let Some(last) = coverage.last_mut() {
                        last.to = last.to.max(to);
                        if last.kind != kind {
                            last.kind = "covered".into();
                        }
                        continue;
                    }
                }
                coverage.push(CoverageRange { from, to, kind });
            }
            out.push(SeriesStatus {
                symbol,
                timeframe,
                bars,
                coverage,
            });
        }
        Ok(out)
    }

    fn only_nontrading_gap(&self, before: NaiveDate, after: NaiveDate) -> Result<bool, DataError> {
        let mut day = before + chrono::Days::new(1);
        while day < after {
            if day.weekday().number_from_monday() <= 5 {
                let is_open: bool = self.conn.query_row(
                    "SELECT COALESCE((SELECT is_trading=1 FROM trading_calendar WHERE date=?1),1)",
                    [day.to_string()],
                    |r| r.get(0),
                )?;
                if is_open {
                    return Ok(false);
                }
            }
            day = day + chrono::Days::new(1);
        }
        Ok(true)
    }

    pub fn quality_rows(&self) -> Result<Vec<QualityRow>, DataError> {
        let mut q = self.conn.prepare(
            "SELECT COALESCE(i.symbol, ''), COALESCE(s.tf, ''), q.ts, q.kind
             FROM quality q
             LEFT JOIN series s ON q.series_id = s.id
             LEFT JOIN instruments i ON s.security_id = i.security_id
             ORDER BY q.id",
        )?;
        let rows = q.query_map([], |r| {
            let symbol: String = r.get(0)?;
            let tf: String = r.get(1)?;
            let ts: Option<i64> = r.get(2)?;
            let kind: String = r.get(3)?;
            let date = match ts {
                Some(t) => DateTime::<Utc>::from_timestamp(t, 0)
                    .map(|dt| dt.with_timezone(&Kolkata).date_naive().to_string())
                    .unwrap_or_default(),
                None => String::new(),
            };
            Ok(QualityRow {
                symbol,
                tf,
                date,
                kind,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn budget_used(&self, date: &str) -> Result<u64, DataError> {
        Ok(self.conn.query_row(
            "SELECT COALESCE((SELECT requests FROM budget WHERE day=?1),0)",
            [date],
            |r| r.get(0),
        )?)
    }

    pub fn budget_add(&self, date: &str, n: u64) -> Result<(), DataError> {
        self.conn.execute("INSERT INTO budget VALUES(?1,?2) ON CONFLICT(day) DO UPDATE SET requests=requests+excluded.requests",params![date,n])?;
        Ok(())
    }

    /// Record (or re-record) a failed request range. `attempts` is incremented
    /// on conflict and `next_retry` advanced so the range is retried on a later
    /// run rather than in a tight loop.
    pub fn record_failure(
        &self,
        id: &str,
        tf: Tf,
        from: NaiveDate,
        to: NaiveDate,
        error_code: &str,
        next_retry: NaiveDate,
    ) -> Result<(), DataError> {
        self.conn.execute(
            "INSERT INTO failures(security_id,tf,from_date,to_date,error_code,attempts,next_retry) VALUES(?1,?2,?3,?4,?5,1,?6) ON CONFLICT(security_id,tf,from_date,to_date) DO UPDATE SET error_code=excluded.error_code,attempts=attempts+1,next_retry=excluded.next_retry",
            params![id, tf.as_str(), from.to_string(), to.to_string(), error_code, next_retry.to_string()],
        )?;
        Ok(())
    }

    /// Clear a recorded failure once its range has been successfully fetched.
    pub fn clear_failure(
        &self,
        id: &str,
        tf: Tf,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<(), DataError> {
        self.conn.execute(
            "DELETE FROM failures WHERE security_id=?1 AND tf=?2 AND from_date=?3 AND to_date=?4",
            params![id, tf.as_str(), from.to_string(), to.to_string()],
        )?;
        Ok(())
    }

    /// Failures whose `next_retry` is on or before `today`, ready to be retried.
    pub fn due_failures(
        &self,
        today: NaiveDate,
    ) -> Result<Vec<(String, Tf, NaiveDate, NaiveDate)>, DataError> {
        let mut q = self.conn.prepare(
            "SELECT security_id,tf,from_date,to_date FROM failures WHERE next_retry<=?1 ORDER BY security_id,tf,from_date",
        )?;
        let rows = q
            .query_map([today.to_string()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let mut out = Vec::with_capacity(rows.len());
        for (id, tf, from, to) in rows {
            let tf =
                Tf::parse(&tf).ok_or_else(|| DataError::Invalid("bad tf in failures".into()))?;
            let from = NaiveDate::parse_from_str(&from, "%Y-%m-%d")
                .map_err(|e| DataError::Invalid(e.to_string()))?;
            let to = NaiveDate::parse_from_str(&to, "%Y-%m-%d")
                .map_err(|e| DataError::Invalid(e.to_string()))?;
            out.push((id, tf, from, to));
        }
        Ok(out)
    }

    pub fn failure_count(&self) -> Result<u64, DataError> {
        Ok(self
            .conn
            .query_row("SELECT count(*) FROM failures", [], |r| r.get(0))?)
    }

    /// A stable fingerprint of the whole catalog's materialised data: every
    /// partition path and checksum, ordered. Two runs that produce identical
    /// coverage produce identical fingerprints (used by `data bulk` summaries
    /// and acceptance tests 3 and 4).
    pub fn catalog_fingerprint(&self) -> Result<String, DataError> {
        let mut q = self
            .conn
            .prepare("SELECT path,checksum FROM partitions ORDER BY series_id,path")?;
        let rows = q
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<Result<Vec<_>, _>>()?;
        let mut h = Sha256::new();
        for (path, sum) in rows {
            h.update(path.as_bytes());
            h.update(b":");
            h.update(sum.as_bytes());
            h.update(b";");
        }
        Ok(format!("sha256:{:x}", h.finalize()))
    }

    /// All catalogued instruments as `(security_id, symbol, instrument)`, ordered
    /// by numeric security id. Used by `data bulk` to walk the universe.
    pub fn instruments_in_order(&self) -> Result<Vec<(String, String, String)>, DataError> {
        let mut q = self.conn.prepare(
            "SELECT security_id,symbol,instrument FROM instruments ORDER BY CAST(security_id AS INTEGER),security_id",
        )?;
        let rows = q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn runs_start(&self) -> Result<i64, DataError> {
        self.conn.execute(
            "INSERT INTO runs(started,status) VALUES(strftime('%s','now'),'running')",
            [],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn runs_finish(&self, id: i64, requests: u64, status: &str) -> Result<(), DataError> {
        self.conn.execute(
            "UPDATE runs SET finished=strftime('%s','now'),requests=?2,status=?3 WHERE id=?1",
            params![id, requests, status],
        )?;
        Ok(())
    }

    pub fn mark_quality(&self, id: &str, tf: Tf, q: &Quality) -> Result<(), DataError> {
        let sid = self.ensure_series(id, tf)?;
        self.conn.execute(
            "INSERT INTO quality(series_id,kind,ts,detail) VALUES(?1,?2,?3,?4)",
            params![sid, q.kind, q.ts, q.detail],
        )?;
        Ok(())
    }
}
