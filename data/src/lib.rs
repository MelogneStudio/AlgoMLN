use algomln::sim::{CandleSource, CandleStream, DateRange, Fingerprint, SourceError, Timeframe};
use algomln::{engine::Symbol, indicators::Candle};
use arrow_array::{Array, Float64Array, Int64Array, RecordBatch};
use arrow_schema::{DataType, Field, Schema};
use chrono::{DateTime, Datelike, NaiveDate, Timelike, Utc};
use chrono_tz::Asia::Kolkata;
use dhan_api::{ApiError, Candle as DhanCandle, DhanClient, Scrip};
use parquet::{
    arrow::{arrow_reader::ParquetRecordBatchReaderBuilder, ArrowWriter},
    basic::Compression,
    file::properties::WriterProperties,
};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Db(#[from] rusqlite::Error),
    #[error("{0}")]
    Api(#[from] ApiError),
    #[error("invalid data: {0}")]
    Invalid(String),
    #[error("uncovered range for {0}; run `algomln data update` first")]
    Uncovered(String),
    #[error("daily request budget reached ({used}/{cap})")]
    BudgetExhausted { used: u64, cap: u64 },
}
impl DataError {
    /// True when the run must abort immediately (bad credentials). Everything
    /// else that reaches the bulk layer is recorded as a failure and skipped.
    pub fn is_auth(&self) -> bool {
        matches!(self, DataError::Api(ApiError::Auth(_)))
    }
    /// True when the daily request budget has been reached.
    pub fn is_budget(&self) -> bool {
        matches!(self, DataError::BudgetExhausted { .. })
    }
    /// A short, stable code suitable for the `failures.error_code` column.
    pub fn code(&self) -> String {
        match self {
            DataError::Api(ApiError::Auth(_)) => "auth".into(),
            DataError::Api(ApiError::RateLimit { code, .. }) => code.clone(),
            DataError::Api(ApiError::DataApi { code, .. }) => code.clone(),
            DataError::Api(ApiError::BadRequest { code, .. }) => code.clone(),
            DataError::Api(ApiError::SevereRateLimit(_)) => "DH-805".into(),
            DataError::Api(ApiError::DataUnavailable(_)) => "DH-907".into(),
            DataError::Api(ApiError::Access(_)) => "access".into(),
            DataError::Api(ApiError::Server(_)) => "server".into(),
            DataError::Api(ApiError::Transport(_)) => "transport".into(),
            DataError::Api(ApiError::Malformed(_)) => "malformed".into(),
            DataError::BudgetExhausted { .. } => "budget".into(),
            DataError::Io(_) => "io".into(),
            DataError::Db(_) => "db".into(),
            DataError::Invalid(_) => "invalid".into(),
            DataError::Uncovered(_) => "uncovered".into(),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tf {
    M1,
    M5,
    M15,
    M25,
    M60,
    D1,
}
impl Tf {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::M1 => "1m",
            Self::M5 => "5m",
            Self::M15 => "15m",
            Self::M25 => "25m",
            Self::M60 => "60m",
            Self::D1 => "1d",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "1m" => Self::M1,
            "5m" => Self::M5,
            "15m" => Self::M15,
            "25m" => Self::M25,
            "60m" => Self::M60,
            "1d" => Self::D1,
            _ => return None,
        })
    }
    fn seconds(self) -> i64 {
        match self {
            Self::M1 => 60,
            Self::M5 => 300,
            Self::M15 => 900,
            Self::M25 => 1500,
            Self::M60 => 3600,
            Self::D1 => 86400,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bar {
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}
impl From<DhanCandle> for Bar {
    fn from(x: DhanCandle) -> Self {
        Self {
            ts: x.ts,
            open: x.open,
            high: x.high,
            low: x.low,
            close: x.close,
            volume: x.volume,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Instrument {
    pub security_id: String,
    pub symbol: String,
    pub segment: String,
    pub lot_size: u32,
    pub tick_size_paise: i64,
    pub refreshed_at: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quality {
    pub kind: String,
    pub ts: Option<i64>,
    pub detail: String,
}
#[derive(Clone, Debug)]
pub struct PartitionMeta {
    pub path: PathBuf,
    pub checksum: String,
    pub first_ts: i64,
    pub last_ts: i64,
    pub rows: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Coverage {
    pub start_ts: i64,
    pub end_ts: i64,
    pub kind: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageRange {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub kind: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeriesStatus {
    pub symbol: String,
    pub timeframe: String,
    pub bars: u64,
    pub coverage: Vec<CoverageRange>,
}

pub struct Catalog {
    conn: Connection,
}
impl Catalog {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DataError> {
        if let Some(p) = path.as_ref().parent() {
            fs::create_dir_all(p)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; CREATE TABLE IF NOT EXISTS instruments(security_id TEXT PRIMARY KEY,symbol TEXT NOT NULL UNIQUE,segment TEXT NOT NULL,lot_size INTEGER NOT NULL,tick_size_paise INTEGER NOT NULL,refreshed_at INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS series(id INTEGER PRIMARY KEY,security_id TEXT NOT NULL,tf TEXT NOT NULL,first_ts INTEGER,last_ts INTEGER,rows INTEGER NOT NULL DEFAULT 0,needs_refresh INTEGER NOT NULL DEFAULT 0,UNIQUE(security_id,tf)); CREATE TABLE IF NOT EXISTS partitions(series_id INTEGER NOT NULL,path TEXT NOT NULL,first_ts INTEGER NOT NULL,last_ts INTEGER NOT NULL,rows INTEGER NOT NULL,checksum TEXT NOT NULL,PRIMARY KEY(series_id,path)); CREATE TABLE IF NOT EXISTS coverage(series_id INTEGER NOT NULL,start_ts INTEGER NOT NULL,end_ts INTEGER NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('data','confirmed_empty'))); CREATE TABLE IF NOT EXISTS sessions(series_id INTEGER NOT NULL,date TEXT NOT NULL,first_ts INTEGER NOT NULL,last_ts INTEGER NOT NULL,bars INTEGER NOT NULL,PRIMARY KEY(series_id,date)); CREATE TABLE IF NOT EXISTS quality(id INTEGER PRIMARY KEY,series_id INTEGER,kind TEXT NOT NULL,ts INTEGER,detail TEXT NOT NULL); CREATE TABLE IF NOT EXISTS runs(id INTEGER PRIMARY KEY,started INTEGER NOT NULL,finished INTEGER,requests INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL); CREATE TABLE IF NOT EXISTS budget(day TEXT PRIMARY KEY,requests INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS trading_calendar(date TEXT PRIMARY KEY,is_trading INTEGER NOT NULL,source TEXT NOT NULL); CREATE TABLE IF NOT EXISTS failures(security_id INTEGER,tf TEXT,from_date TEXT,to_date TEXT,error_code TEXT,attempts INTEGER,next_retry TEXT,PRIMARY KEY(security_id,tf,from_date,to_date));")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 1 {
            return Err(DataError::Invalid(format!(
                "catalog schema version {version} is newer than supported version 1"
            )));
        }
        let has_instrument: bool = {
            let mut q = conn.prepare("PRAGMA table_info(instruments)")?;
            let names = q
                .query_map([], |r| r.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()?;
            names.iter().any(|x| x == "instrument")
        };
        if !has_instrument {
            conn.execute(
                "ALTER TABLE instruments ADD COLUMN instrument TEXT NOT NULL DEFAULT 'EQUITY'",
                [],
            )?;
        }
        conn.execute_batch("PRAGMA user_version=1;")?;
        Ok(Self { conn })
    }
    pub fn refresh_instruments(&mut self, rows: &[Scrip], at: i64) -> Result<(), DataError> {
        let tx = self.conn.transaction()?;
        for x in rows {
            tx.execute("INSERT INTO instruments(security_id,symbol,segment,lot_size,tick_size_paise,instrument,refreshed_at) VALUES(?1,?2,'NSE_EQ',?3,?4,?5,?6) ON CONFLICT(security_id) DO UPDATE SET symbol=excluded.symbol,lot_size=excluded.lot_size,tick_size_paise=excluded.tick_size_paise,instrument=excluded.instrument,refreshed_at=excluded.refreshed_at",params![x.security_id,x.symbol,x.lot_size,x.tick_size_paise,x.instrument,at])?;
        }
        tx.commit()?;
        Ok(())
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
        self.series(id, tf)
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
    fn series(&self, id: &str, tf: Tf) -> Result<i64, DataError> {
        self.conn.execute(
            "INSERT OR IGNORE INTO series(security_id,tf) VALUES(?1,?2)",
            params![id, tf.as_str()],
        )?;
        Ok(self.conn.query_row(
            "SELECT id FROM series WHERE security_id=?1 AND tf=?2",
            params![id, tf.as_str()],
            |r| r.get(0),
        )?)
    }
    pub fn has_coverage(&self, id: &str, tf: Tf, from: i64, to: i64) -> Result<bool, DataError> {
        let sid = self.series(id, tf)?;
        let first = DateTime::<Utc>::from_timestamp(from, 0)
            .unwrap_or(DateTime::UNIX_EPOCH)
            .with_timezone(&Kolkata)
            .date_naive();
        let last = DateTime::<Utc>::from_timestamp(to, 0)
            .unwrap_or(DateTime::UNIX_EPOCH)
            .with_timezone(&Kolkata)
            .date_naive();
        let a = local_midnight(first);
        let b = local_midnight(last + chrono::Days::new(1)) - 1;
        let mut stmt=self.conn.prepare("SELECT start_ts,end_ts FROM coverage WHERE series_id=?1 AND end_ts>=?2 AND start_ts<=?3 ORDER BY start_ts")?;
        let rows = stmt.query_map(params![sid, a, b], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
        })?;
        let mut cursor = a;
        for row in rows {
            let (lo, hi) = row?;
            if lo > cursor {
                return Ok(false);
            }
            cursor = cursor.max(hi.saturating_add(1));
            if cursor > b {
                return Ok(true);
            }
        }
        Ok(false)
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
        let sid = self.series(id, tf)?;
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
        let sid = self.series(id, tf)?;
        let mut q=self.conn.prepare("SELECT date FROM sessions WHERE series_id=?1 AND date>=?2 AND date<=?3 UNION SELECT date FROM trading_calendar WHERE is_trading=0 AND date>=?2 AND date<=?3 ORDER BY date")?;
        let xs = q
            .query_map(params![sid, from.to_string(), to.to_string()], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<Result<Vec<_>, _>>()?;
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
        let sid = self.series(id, tf)?;
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
    pub fn quality_rows(&self) -> Result<Vec<(String, String)>, DataError> {
        let mut q = self
            .conn
            .prepare("SELECT kind,detail FROM quality ORDER BY id")?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
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
            let tf = Tf::parse(&tf).ok_or_else(|| DataError::Invalid("bad tf in failures".into()))?;
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
        let sid = self.series(id, tf)?;
        self.conn.execute(
            "INSERT INTO quality(series_id,kind,ts,detail) VALUES(?1,?2,?3,?4)",
            params![sid, q.kind, q.ts, q.detail],
        )?;
        Ok(())
    }
}

pub struct Store {
    root: PathBuf,
}
impl Store {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    fn partition(&self, id: &str, tf: Tf, ts: i64) -> PathBuf {
        let dt = DateTime::<Utc>::from_timestamp(ts, 0)
            .unwrap_or(DateTime::UNIX_EPOCH)
            .with_timezone(&Kolkata);
        let period = if tf == Tf::D1 {
            dt.format("%Y").to_string()
        } else {
            dt.format("%Y-%m").to_string()
        };
        self.root
            .join("candles")
            .join(id)
            .join(tf.as_str())
            .join(format!("{period}.parquet"))
    }
    pub fn write_partitions(
        &self,
        id: &str,
        tf: Tf,
        bars: &[Bar],
    ) -> Result<Vec<PartitionMeta>, DataError> {
        let mut groups: BTreeMap<PathBuf, Vec<Bar>> = BTreeMap::new();
        for b in bars {
            groups
                .entry(self.partition(id, tf, b.ts))
                .or_default()
                .push(b.clone());
        }
        let mut out = Vec::new();
        for (path, chunk) in groups {
            let (sum, first, last, n) = self.write_one(&path, &chunk)?;
            out.push(PartitionMeta {
                path,
                checksum: sum,
                first_ts: first,
                last_ts: last,
                rows: n,
            });
        }
        Ok(out)
    }
    fn write_one(&self, path: &Path, bars: &[Bar]) -> Result<(String, i64, i64, usize), DataError> {
        if let Some(p) = path.parent() {
            fs::create_dir_all(p)?;
        }
        let existing = if path.exists() {
            read_bars(path)?
        } else {
            Vec::new()
        };
        let existing_map: HashMap<i64, &Bar> = existing.iter().map(|b| (b.ts, b)).collect();
        if !existing.is_empty()
            && bars
                .iter()
                .all(|b| existing_map.get(&b.ts).is_some_and(|old| *old == b))
        {
            let sum = checksum(path)?;
            return Ok((
                sum,
                existing.first().unwrap().ts,
                existing.last().unwrap().ts,
                existing.len(),
            ));
        }
        let mut map: BTreeMap<i64, Bar> = existing.into_iter().map(|b| (b.ts, b)).collect();
        for b in bars {
            map.insert(b.ts, b.clone());
        }
        let rows: Vec<_> = map.into_values().collect();
        let schema = Arc::new(Schema::new(vec![
            Field::new("ts", DataType::Int64, false),
            Field::new("open", DataType::Float64, false),
            Field::new("high", DataType::Float64, false),
            Field::new("low", DataType::Float64, false),
            Field::new("close", DataType::Float64, false),
            Field::new("volume", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(
                    rows.iter().map(|x| x.ts).collect::<Vec<_>>(),
                )),
                Arc::new(Float64Array::from(
                    rows.iter().map(|x| x.open).collect::<Vec<_>>(),
                )),
                Arc::new(Float64Array::from(
                    rows.iter().map(|x| x.high).collect::<Vec<_>>(),
                )),
                Arc::new(Float64Array::from(
                    rows.iter().map(|x| x.low).collect::<Vec<_>>(),
                )),
                Arc::new(Float64Array::from(
                    rows.iter().map(|x| x.close).collect::<Vec<_>>(),
                )),
                Arc::new(Float64Array::from(
                    rows.iter().map(|x| x.volume).collect::<Vec<_>>(),
                )),
            ],
        )
        .map_err(|e| DataError::Invalid(e.to_string()))?;
        let tmp = path.with_extension(format!(
            "parquet.tmp-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let file = File::create(&tmp)?;
        let sync_file = file.try_clone()?;
        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();
        let mut writer = ArrowWriter::try_new(file, schema, Some(props))
            .map_err(|e| DataError::Invalid(e.to_string()))?;
        writer
            .write(&batch)
            .map_err(|e| DataError::Invalid(e.to_string()))?;
        writer
            .close()
            .map_err(|e| DataError::Invalid(e.to_string()))?;
        sync_file.sync_all()?;
        atomic_replace(&tmp, path)?;
        let sum = checksum(path)?;
        Ok((
            sum,
            rows.first().unwrap().ts,
            rows.last().unwrap().ts,
            rows.len(),
        ))
    }
    pub fn reconcile(&self, cat: &mut Catalog) -> Result<(), DataError> {
        let mut files = HashMap::new();
        let base = self.root.join("candles");
        if base.exists() {
            for p in walk(&base)? {
                if p.extension().and_then(|x| x.to_str()) == Some("parquet") {
                    files.insert(p.clone(), checksum(&p)?);
                }
            }
        }
        let tx = cat.conn.transaction()?;
        let mut q = tx.prepare("SELECT p.series_id,p.path,p.checksum FROM partitions p")?;
        let entries = q
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(q);
        let mut affected = std::collections::HashSet::new();
        for (sid, rel, hash) in entries {
            let p = self.root.join(&rel);
            match files.remove(&p) {
                Some(found) if found == hash => {}
                Some(_) | None => {
                    affected.insert(sid);
                    tx.execute(
                        "DELETE FROM partitions WHERE series_id=?1 AND path=?2",
                        params![sid, rel],
                    )?;
                    tx.execute("DELETE FROM coverage WHERE series_id=?1", [sid])?;
                    tx.execute("DELETE FROM sessions WHERE series_id=?1", [sid])?;
                    tx.execute(
                        "UPDATE series SET first_ts=NULL,last_ts=NULL,rows=0 WHERE id=?1",
                        [sid],
                    )?;
                    tx.execute("INSERT INTO quality(series_id,kind,detail) SELECT ?1,'reconcile_missing_or_checksum','partition removed from catalog coverage' WHERE NOT EXISTS(SELECT 1 FROM quality WHERE series_id=?1 AND kind='reconcile_missing_or_checksum')",[sid])?;
                }
            }
        }
        for (orphan, _) in files {
            tx.execute("INSERT INTO quality(kind,detail) SELECT 'reconcile_orphan_file',?1 WHERE NOT EXISTS(SELECT 1 FROM quality WHERE kind='reconcile_orphan_file' AND detail=?1)",[orphan.to_string_lossy().to_string()])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn remove_series_files(&self, id: &str, tf: Tf) -> Result<(), DataError> {
        let dir = self.root.join("candles").join(id).join(tf.as_str());
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }
}

#[cfg(not(windows))]
fn atomic_replace(from: &Path, to: &Path) -> Result<(), DataError> {
    fs::rename(from, to)?;
    Ok(())
}
#[cfg(windows)]
fn atomic_replace(from: &Path, to: &Path) -> Result<(), DataError> {
    use std::os::windows::ffi::OsStrExt;
    let src: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let dst: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            src.as_ptr(),
            dst.as_ptr(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
fn checksum(p: &Path) -> Result<String, DataError> {
    let bytes = fs::read(p)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}
fn walk(root: &Path) -> Result<Vec<PathBuf>, DataError> {
    let mut v = Vec::new();
    for e in fs::read_dir(root)? {
        let p = e?.path();
        if p.is_dir() {
            v.extend(walk(&p)?)
        } else {
            v.push(p)
        }
    }
    Ok(v)
}
fn read_bars(path: &Path) -> Result<Vec<Bar>, DataError> {
    let f = File::open(path)?;
    let r = ParquetRecordBatchReaderBuilder::try_new(f)
        .map_err(|e| DataError::Invalid(e.to_string()))?
        .build()
        .map_err(|e| DataError::Invalid(e.to_string()))?;
    let mut v = Vec::new();
    for batch in r {
        let b = batch.map_err(|e| DataError::Invalid(e.to_string()))?;
        let ts = b
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| DataError::Invalid("ts schema".into()))?;
        let cols: Vec<_> = (1..6)
            .map(|i| {
                b.column(i)
                    .as_any()
                    .downcast_ref::<Float64Array>()
                    .ok_or_else(|| DataError::Invalid("OHLCV schema".into()))
            })
            .collect::<Result<_, _>>()?;
        for i in 0..b.num_rows() {
            v.push(Bar {
                ts: ts.value(i),
                open: cols[0].value(i),
                high: cols[1].value(i),
                low: cols[2].value(i),
                close: cols[3].value(i),
                volume: cols[4].value(i),
            });
        }
    }
    Ok(v)
}

pub fn missing_date_ranges(
    cat: &Catalog,
    id: &str,
    tf: Tf,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<(NaiveDate, NaiveDate)>, DataError> {
    let missing = cat.missing_dates(id, tf, from, to)?;
    Ok(match (missing.first(), missing.last()) {
        (Some(a), Some(b)) => vec![(*a, *b)],
        _ => Vec::new(),
    })
}
fn local_midnight(d: NaiveDate) -> i64 {
    d.and_hms_opt(0, 0, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp()
}
pub fn chunk_dates(from: NaiveDate, to: NaiveDate) -> Vec<(NaiveDate, NaiveDate)> {
    chunk_dates_limited(from, to, 89)
}
pub fn chunk_dates_limited(
    from: NaiveDate,
    to: NaiveDate,
    days: u64,
) -> Vec<(NaiveDate, NaiveDate)> {
    let mut out = Vec::new();
    let mut s = from;
    while s <= to {
        let e = (s + chrono::Days::new(days.saturating_sub(1))).min(to);
        out.push((s, e));
        s = e + chrono::Days::new(1);
    }
    out
}
pub fn last_complete_bar(now: i64, tf: Tf, _session_end: i64, margin: i64) -> i64 {
    let step = tf.seconds();
    let local = DateTime::<Utc>::from_timestamp(now, 0)
        .unwrap_or(DateTime::UNIX_EPOCH)
        .with_timezone(&Kolkata);
    if tf == Tf::D1 {
        let cutoff = local
            .date_naive()
            .and_hms_opt(16, 0, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        return if now >= cutoff {
            cutoff - 33300
        } else {
            (local.date_naive() - chrono::Days::new(1))
                .and_hms_opt(9, 15, 0)
                .unwrap()
                .and_local_timezone(Kolkata)
                .single()
                .unwrap()
                .timestamp()
                - 33300
        };
    }
    let last = now - margin - step;
    last - last.rem_euclid(step)
}
pub fn complete_intraday(
    bars: Vec<Bar>,
    now: i64,
    tf: Tf,
    session_ends: &HashMap<String, i64>,
    margin: i64,
) -> Vec<Bar> {
    bars.into_iter()
        .filter(|b| {
            let local = DateTime::<Utc>::from_timestamp(b.ts, 0)
                .unwrap_or(DateTime::UNIX_EPOCH)
                .with_timezone(&Kolkata);
            let end = session_ends
                .get(&local.date_naive().to_string())
                .copied()
                .unwrap_or_else(|| {
                    local
                        .date_naive()
                        .and_hms_opt(15, 30, 0)
                        .unwrap()
                        .and_local_timezone(Kolkata)
                        .single()
                        .unwrap()
                        .timestamp()
                });
            now >= ((b.ts + tf.seconds()).min(end) + margin)
        })
        .collect()
}
pub fn session_shapes(bars: &[Bar]) -> BTreeMap<NaiveDate, (i64, i64, usize)> {
    let mut m: BTreeMap<NaiveDate, Vec<i64>> = BTreeMap::new();
    for b in bars {
        let d = DateTime::<Utc>::from_timestamp(b.ts, 0)
            .unwrap_or(DateTime::UNIX_EPOCH)
            .with_timezone(&Kolkata)
            .date_naive();
        m.entry(d).or_default().push(b.ts);
    }
    m.into_iter()
        .map(|(d, mut xs)| {
            xs.sort();
            (d, (xs[0], *xs.last().unwrap(), xs.len()))
        })
        .collect()
}
pub fn session_regime_changes(shapes: &BTreeMap<NaiveDate, (i64, i64, usize)>) -> Vec<NaiveDate> {
    let mut out = Vec::new();
    let mut history = Vec::<i64>::new();
    let mut prior = None;
    for (d, (_, last, _)) in shapes {
        let local = DateTime::<Utc>::from_timestamp(*last, 0)
            .unwrap_or(DateTime::UNIX_EPOCH)
            .with_timezone(&Kolkata);
        let value = local.hour() as i64 * 3600 + local.minute() as i64 * 60 + local.second() as i64;
        history.push(value);
        if history.len() > 20 {
            history.remove(0);
        }
        let mut counts = HashMap::<i64, usize>::new();
        for x in &history {
            *counts.entry(*x).or_default() += 1;
        }
        let mode = counts
            .into_iter()
            .max_by_key(|(v, n)| (*n, std::cmp::Reverse(*v)))
            .map(|(v, _)| v);
        if let (Some(old), Some(new)) = (prior, mode) {
            if old != new {
                out.push(*d)
            }
        }
        prior = mode;
    }
    out
}

fn instrument_interval(tf: Tf) -> &'static str {
    match tf {
        Tf::M1 => "1",
        Tf::M5 => "5",
        Tf::M15 => "15",
        Tf::M25 => "25",
        Tf::M60 => "60",
        Tf::D1 => "day",
    }
}
pub fn check_overlap(old: &[Bar], new: &[Bar]) -> (&'static str, Vec<i64>) {
    let oldm: HashMap<_, _> = old.iter().map(|b| (b.ts, b)).collect();
    let mut diffs = Vec::new();
    let mut ratios = Vec::new();
    let mut overlap = 0usize;
    for n in new {
        if let Some(o) = oldm.get(&n.ts) {
            overlap += 1;
            if o != &n {
                diffs.push(n.ts);
            }
            if n.close != 0.0 {
                ratios.push(o.close / n.close);
            }
        }
    }
    if diffs.is_empty() {
        return ("identical", diffs);
    }
    let avg = ratios.iter().sum::<f64>() / ratios.len().max(1) as f64;
    if ratios.len() == overlap
        && ratios.iter().all(|x| (x - avg).abs() < 1e-6)
        && (avg - 1.0).abs() > 1e-4
    {
        return ("corporate_action", diffs);
    }
    ("revised", diffs)
}

pub trait HistoryApi: Send + Sync {
    fn intraday(
        &self,
        id: &str,
        instrument: &str,
        interval: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Bar>, DataError>;
    fn daily(
        &self,
        id: &str,
        instrument: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Bar>, DataError>;
}
impl HistoryApi for DhanClient {
    fn intraday(
        &self,
        id: &str,
        instrument: &str,
        interval: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Bar>, DataError> {
        Ok(
            DhanClient::intraday(self, id, instrument, interval, from, to)?
                .into_iter()
                .map(Into::into)
                .collect(),
        )
    }
    fn daily(
        &self,
        id: &str,
        instrument: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Bar>, DataError> {
        Ok(DhanClient::daily(self, id, instrument, from, to)?
            .into_iter()
            .map(Into::into)
            .collect())
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
pub const DEFAULT_BUDGET_CAP: u64 = 90_000;
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
        let today = chrono::Utc::now().date_naive().to_string();
        self.budget_guard(&today)?;
        let run = self.catalog.runs_start()?;
        let mut requests = 0;
        let result = (|| {
            self.store.reconcile(&mut self.catalog)?;
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
                    let existing_all: Vec<Bar> = {
                        let sid = self.catalog.series(security_id, tf)?;
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
                    let sid = self.catalog.series(security_id, tf)?;
                    let tx = self.catalog.conn.transaction()?;
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
                                        let sid = self.catalog.series(security_id, tf)?;
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
impl<F: Fn() -> Box<dyn HistoryApi>> BulkDownloader<F> {
    fn today(&self) -> String {
        chrono::Utc::now().date_naive().to_string()
    }
    /// Resolve the processing order: priority symbols first (in file order),
    /// then every other instrument in security-id order, de-duplicated.
    fn order(&self, cat: &Catalog) -> Result<Vec<(String, String, String)>, DataError> {
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
    pub fn run<P: FnMut(&Progress)>(&self, mut progress: P) -> Result<BulkReport, DataError> {
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
            let cat = Catalog::open(&self.catalog_path)?;
            report.failures = cat.failure_count()?;
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
                let outcome = updater.update(id, *tf, instrument, self.config.from, self.config.to);
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
fn format_eta(secs: i64) -> String {
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
fn group_thousands(n: u64) -> String {
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

pub struct CatalogSource {
    pub store: Store,
    pub catalog_path: PathBuf,
    pub allow_gaps: bool,
}
impl CatalogSource {
    pub fn new(store: Store, catalog_path: impl Into<PathBuf>) -> Self {
        Self {
            store,
            catalog_path: catalog_path.into(),
            allow_gaps: false,
        }
    }
    fn catalog(&self) -> Result<Catalog, DataError> {
        Catalog::open(&self.catalog_path)
    }
    pub fn reconcile(&self) -> Result<(), DataError> {
        let mut catalog = self.catalog()?;
        self.store.reconcile(&mut catalog)
    }
}
impl CandleSource for CatalogSource {
    fn open(
        &self,
        symbols: &[Symbol],
        timeframe: Timeframe,
        range: DateRange,
    ) -> Result<CandleStream, SourceError> {
        self.reconcile()
            .map_err(|e| SourceError::Other(format!("catalog reconciliation failed: {e}")))?;
        let catalog = self
            .catalog()
            .map_err(|e| SourceError::Other(e.to_string()))?;
        let tf = Tf::parse(timeframe.as_str())
            .ok_or_else(|| SourceError::Other("unsupported timeframe".into()))?;
        let (start, end) = (range.from.unwrap_or(i64::MIN), range.to.unwrap_or(i64::MAX));
        let mut streams = Vec::<Vec<(Symbol, Candle)>>::new();
        for symbol in symbols {
            let (id, _) = catalog
                .resolve(symbol)
                .map_err(|e| SourceError::Other(e.to_string()))?;
            if !catalog
                .has_coverage(&id, tf, start, end)
                .map_err(|e| SourceError::Other(e.to_string()))?
            {
                if self.allow_gaps {
                    continue;
                }
                return Err(SourceError::Other(format!(
                    "{}; run `algomln data update --symbols {} --tf {} --from <from> --to <to>`",
                    DataError::Uncovered(symbol.clone()),
                    symbol,
                    timeframe.as_str()
                )));
            }
            let sid = catalog
                .series(&id, tf)
                .map_err(|e| SourceError::Other(e.to_string()))?;
            let mut stmt = catalog
                .conn
                .prepare("SELECT path FROM partitions WHERE series_id=?1 ORDER BY first_ts")
                .map_err(|e| SourceError::Other(e.to_string()))?;
            let files = stmt
                .query_map([sid], |r| r.get::<_, String>(0))
                .map_err(|e| SourceError::Other(e.to_string()))?;
            let mut symbol_rows = Vec::new();
            for f in files {
                let path = self
                    .store
                    .root
                    .join(f.map_err(|e| SourceError::Other(e.to_string()))?);
                for b in read_bars(&path).map_err(|e| SourceError::Other(e.to_string()))? {
                    if (start..=end).contains(&b.ts) {
                        symbol_rows.push((
                            symbol.clone(),
                            Candle {
                                timestamp: b.ts,
                                open: b.open,
                                high: b.high,
                                low: b.low,
                                close: b.close,
                                volume: b.volume,
                            },
                        ));
                    }
                }
            }
            symbol_rows.sort_by_key(|x| x.1.timestamp);
            streams.push(symbol_rows);
        }
        Ok(Box::new(KWayMerge::new(streams)))
    }
    fn instrument_rules(
        &self,
        symbols: &[Symbol],
    ) -> BTreeMap<Symbol, algomln::trader::InstrumentRule> {
        let Ok(c) = self.catalog() else {
            return BTreeMap::new();
        };
        let mut out = BTreeMap::new();
        for s in symbols {
            if let Ok((id, _)) = c.resolve(s) {
                if let Ok(rule) = c.conn.query_row(
                    "SELECT lot_size FROM instruments WHERE security_id=?1",
                    [id],
                    |r| r.get::<_, u64>(0),
                ) {
                    out.insert(
                        s.clone(),
                        algomln::trader::InstrumentRule {
                            lot_size: rule.max(1),
                            min_qty: 1,
                        },
                    );
                }
            }
        }
        out
    }
    fn tick_sizes_paise(&self, symbols: &[Symbol]) -> BTreeMap<Symbol, i64> {
        let Ok(c) = self.catalog() else {
            return BTreeMap::new();
        };
        let mut out = BTreeMap::new();
        for s in symbols {
            if let Ok((id, _)) = c.resolve(s) {
                if let Ok(t) = c.conn.query_row(
                    "SELECT tick_size_paise FROM instruments WHERE security_id=?1",
                    [id],
                    |r| r.get::<_, i64>(0),
                ) {
                    out.insert(s.clone(), t.max(1));
                }
            }
        }
        out
    }
    fn fingerprint(&self, symbols: &[Symbol], tf: Timeframe, range: DateRange) -> Fingerprint {
        let Ok(c) = self.catalog() else {
            return Fingerprint("catalog-unavailable".into());
        };
        let mut h = Sha256::new();
        for s in symbols {
            let _ = write!(
                h,
                "{s}:{}:{}:{};",
                tf.as_str(),
                range.from.unwrap_or(i64::MIN),
                range.to.unwrap_or(i64::MAX)
            );
            if let Ok((id, _)) = c.resolve(s) {
                if let Some(t) = Tf::parse(tf.as_str()) {
                    if let Ok(sid) = c.series(&id, t) {
                        if let Ok(mut q) = c.conn.prepare(
                            "SELECT path,checksum FROM partitions WHERE series_id=?1 ORDER BY path",
                        ) {
                            if let Ok(it) = q.query_map([sid], |r| {
                                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                            }) {
                                for x in it.flatten() {
                                    let _ = write!(h, "{}:{}", x.0, x.1);
                                }
                            }
                        }
                    }
                }
            }
        }
        Fingerprint(format!("sha256:{:x}", h.finalize()))
    }
}

struct KWayMerge {
    streams: Vec<Vec<(Symbol, Candle)>>,
    positions: Vec<usize>,
    heap: std::collections::BinaryHeap<std::cmp::Reverse<(i64, Symbol, usize)>>,
}
impl KWayMerge {
    fn new(streams: Vec<Vec<(Symbol, Candle)>>) -> Self {
        let mut heap = std::collections::BinaryHeap::new();
        let positions = vec![0; streams.len()];
        for (i, s) in streams.iter().enumerate() {
            if let Some((sym, c)) = s.first() {
                heap.push(std::cmp::Reverse((c.timestamp, sym.clone(), i)));
            }
        }
        Self {
            streams,
            positions,
            heap,
        }
    }
}
impl Iterator for KWayMerge {
    type Item = Result<(Symbol, Candle), SourceError>;
    fn next(&mut self) -> Option<Self::Item> {
        let std::cmp::Reverse((_, _, i)) = self.heap.pop()?;
        let pos = self.positions[i];
        let item = self.streams[i][pos].clone();
        self.positions[i] += 1;
        if let Some((sym, c)) = self.streams[i].get(self.positions[i]) {
            self.heap
                .push(std::cmp::Reverse((c.timestamp, sym.clone(), i)));
        }
        Some(Ok(item))
    }
}

pub fn read_partition(path: &Path) -> Result<Vec<Bar>, DataError> {
    read_bars(path)
}
pub fn validate_bar(b: &Bar) -> bool {
    [b.open, b.high, b.low, b.close, b.volume]
        .iter()
        .all(|x| x.is_finite())
        && b.volume >= 0.0
        && b.low <= b.open.min(b.close)
        && b.high >= b.open.max(b.close)
        && b.ts.rem_euclid(60) == 0
}
pub fn drop_in_progress(bars: Vec<Bar>, now: i64, step: i64) -> Vec<Bar> {
    bars.into_iter().filter(|b| now >= b.ts + step).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct FakeHistory {
        calls: Arc<AtomicUsize>,
        empty_first: bool,
    }
    struct CountingHistory {
        calls: Arc<AtomicUsize>,
    }
    impl HistoryApi for CountingHistory {
        fn intraday(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: NaiveDate,
            _: NaiveDate,
        ) -> Result<Vec<Bar>, DataError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
        fn daily(
            &self,
            _: &str,
            _: &str,
            _: NaiveDate,
            _: NaiveDate,
        ) -> Result<Vec<Bar>, DataError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
    }
    impl HistoryApi for FakeHistory {
        fn intraday(
            &self,
            _: &str,
            _: &str,
            _: &str,
            from: NaiveDate,
            _: NaiveDate,
        ) -> Result<Vec<Bar>, DataError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.empty_first && call == 0 {
                return Ok(Vec::new());
            }
            let ts = from
                .and_hms_opt(9, 15, 0)
                .unwrap()
                .and_local_timezone(Kolkata)
                .single()
                .unwrap()
                .timestamp();
            Ok(vec![bar(ts, 100.0)])
        }
        fn daily(
            &self,
            _: &str,
            _: &str,
            from: NaiveDate,
            _: NaiveDate,
        ) -> Result<Vec<Bar>, DataError> {
            self.intraday("", "", "", from, from)
        }
    }
    fn bar(ts: i64, close: f64) -> Bar {
        Bar {
            ts,
            open: close,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1.0,
        }
    }
    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "algomln-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        ))
    }
    #[test]
    fn chunks_are_89_inclusive_and_daily_fallback_is_configurable() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let xs = chunk_dates(d, d + chrono::Days::new(179));
        assert_eq!(xs.len(), 3);
        assert_eq!((xs[0].1 - xs[0].0).num_days() + 1, 89);
        assert_eq!(
            chunk_dates_limited(d, d + chrono::Days::new(10), 5).len(),
            3
        );
    }
    #[test]
    fn planner_uses_89_calendar_day_chunks_and_one_daily_request() {
        let root = temp("planner-count");
        let db = root.join("catalog.sqlite");
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(
            &[Scrip {
                security_id: "1".into(),
                symbol: "ABC".into(),
                lot_size: 1,
                tick_size_paise: 5,
                instrument: "EQUITY".into(),
            }],
            0,
        )
        .unwrap();
        let from = NaiveDate::from_ymd_opt(2021, 10, 31).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        assert_eq!(chunk_dates(from, to).len(), 21);
        assert_eq!(
            missing_date_ranges(&cat, "1", Tf::M1, from, to).unwrap(),
            vec![(from + chrono::Days::new(1), to - chrono::Days::new(1))]
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let mut update = Updater {
            client: Box::new(CountingHistory {
                calls: calls.clone(),
            }),
            catalog: Catalog::open(&db).unwrap(),
            store: Store::new(&root),
            max_retries: 0,
            margin_seconds: 30,
            reference_symbol_id: None,
            budget_cap: DEFAULT_BUDGET_CAP,
        };
        update.update("1", Tf::M1, "EQUITY", from, to).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 21);
        let calls_d = Arc::new(AtomicUsize::new(0));
        let mut daily = Updater {
            client: Box::new(CountingHistory {
                calls: calls_d.clone(),
            }),
            catalog: Catalog::open(&db).unwrap(),
            store: Store::new(&root),
            max_retries: 0,
            margin_seconds: 30,
            reference_symbol_id: None,
            budget_cap: DEFAULT_BUDGET_CAP,
        };
        daily.update("1", Tf::D1, "EQUITY", from, to).unwrap();
        assert_eq!(calls_d.load(Ordering::SeqCst), 1);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn planner_merges_middle_gaps_over_covered_week_and_weekend() {
        let root = temp("planner-gaps");
        let mut cat = Catalog::open(root.join("catalog.sqlite")).unwrap();
        cat.refresh_instruments(
            &[Scrip {
                security_id: "1".into(),
                symbol: "ABC".into(),
                lot_size: 1,
                tick_size_paise: 5,
                instrument: "EQUITY".into(),
            }],
            0,
        )
        .unwrap();
        let from = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
        for n in 0..=(to - from).num_days() {
            let day = from + chrono::Days::new(n as u64);
            if day.weekday().number_from_monday() <= 5 && day != from && day != to {
                cat.mark_session_empty("1", Tf::M1, day).unwrap();
            }
        }
        assert_eq!(
            missing_date_ranges(&cat, "1", Tf::M1, from, to).unwrap(),
            vec![(from, to)]
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let mut updater = Updater {
            client: Box::new(CountingHistory {
                calls: calls.clone(),
            }),
            catalog: Catalog::open(root.join("catalog.sqlite")).unwrap(),
            store: Store::new(&root),
            max_retries: 0,
            margin_seconds: 0,
            reference_symbol_id: None,
            budget_cap: DEFAULT_BUDGET_CAP,
        };
        updater.update("1", Tf::M1, "EQUITY", from, to).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let middle = from + chrono::Days::new(1);
        let gap_from = from;
        let gap_to = to;
        let mut second = Catalog::open(root.join("other.sqlite")).unwrap();
        second
            .refresh_instruments(
                &[Scrip {
                    security_id: "2".into(),
                    symbol: "DEF".into(),
                    lot_size: 1,
                    tick_size_paise: 5,
                    instrument: "EQUITY".into(),
                }],
                0,
            )
            .unwrap();
        for n in 0..=(gap_to - gap_from).num_days() {
            let day = gap_from + chrono::Days::new(n as u64);
            if day.weekday().number_from_monday() <= 5 && day != middle {
                second.mark_session_empty("2", Tf::M1, day).unwrap();
            }
        }
        let calls = Arc::new(AtomicUsize::new(0));
        let mut updater = Updater {
            client: Box::new(CountingHistory {
                calls: calls.clone(),
            }),
            catalog: second,
            store: Store::new(root.join("other-store")),
            max_retries: 0,
            margin_seconds: 0,
            reference_symbol_id: None,
            budget_cap: DEFAULT_BUDGET_CAP,
        };
        updater
            .update("2", Tf::M1, "EQUITY", gap_from, gap_to)
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn overlap_classifies_split_ratio_and_revision() {
        let a = vec![bar(60, 100.0), bar(120, 200.0)];
        let split = vec![bar(60, 50.0), bar(120, 100.0)];
        assert_eq!(check_overlap(&a, &split).0, "corporate_action");
        let near = vec![bar(60, 99.0), bar(120, 198.0)];
        assert_eq!(check_overlap(&a, &near).0, "corporate_action");
        let revision = vec![bar(60, 101.0), bar(120, 200.0)];
        assert_eq!(check_overlap(&a, &revision).0, "revised");
        assert_eq!(check_overlap(&a, &a).0, "identical");
    }
    #[test]
    fn updater_is_idempotent_and_partial_retry_resumes_coverage() {
        let root = temp("updater");
        let db = root.join("catalog.sqlite");
        let from = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let to = from + chrono::Days::new(1);
        let fake = Arc::new(FakeHistory {
            calls: Arc::new(AtomicUsize::new(0)),
            empty_first: false,
        });
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(
            &[Scrip {
                security_id: "7".into(),
                symbol: "ABC".into(),
                lot_size: 1,
                tick_size_paise: 5,
                instrument: "EQUITY".into(),
            }],
            0,
        )
        .unwrap();
        drop(cat);
        let mut updater = Updater {
            client: Box::new(FakeHistory {
                calls: fake.calls.clone(),
                empty_first: fake.empty_first,
            }),
            catalog: Catalog::open(&db).unwrap(),
            store: Store::new(&root),
            max_retries: 0,
            margin_seconds: 0,
            reference_symbol_id: Some("7".into()),
            budget_cap: DEFAULT_BUDGET_CAP,
        };
        assert!(updater.update("7", Tf::M1, "EQUITY", from, to).unwrap());
        let after_first = fake.calls.load(Ordering::SeqCst);
        assert!(after_first >= 1);
        let covered = updater
            .catalog
            .covered_dates("7", Tf::M1, from, to)
            .unwrap();
        assert_eq!(covered.len(), 2);
        let status = updater.catalog.status_rows().unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].symbol, "ABC");
        assert_eq!(status[0].bars, 1);
        assert_eq!(status[0].coverage.len(), 1);
        assert!(!updater.update("7", Tf::M1, "EQUITY", from, to).unwrap());
        assert_eq!(fake.calls.load(Ordering::SeqCst), after_first);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn confirmed_empty_calendar_dates_are_idempotent_and_cover_sessions() {
        let root = temp("calendar");
        let mut c = Catalog::open(root.join("catalog.sqlite")).unwrap();
        c.refresh_instruments(
            &[Scrip {
                security_id: "1".into(),
                symbol: "ABC".into(),
                lot_size: 1,
                tick_size_paise: 5,
                instrument: "EQUITY".into(),
            }],
            0,
        )
        .unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 1, 26).unwrap();
        c.mark_session_empty("1", Tf::M1, day).unwrap();
        c.mark_session_empty("1", Tf::M1, day).unwrap();
        assert_eq!(c.covered_dates("1", Tf::M1, day, day).unwrap(), vec![day]);
        assert!(c.missing_dates("1", Tf::M1, day, day).unwrap().is_empty());
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn historical_daily_bars_are_not_filtered_by_current_day_cutoff() {
        let root = temp("daily-history");
        let db = root.join("catalog.sqlite");
        let d = NaiveDate::from_ymd_opt(2020, 1, 2).unwrap();
        let ts = d
            .and_hms_opt(9, 15, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(
            &[Scrip {
                security_id: "5".into(),
                symbol: "DAILY".into(),
                lot_size: 1,
                tick_size_paise: 5,
                instrument: "EQUITY".into(),
            }],
            0,
        )
        .unwrap();
        drop(cat);
        struct DailyFake(i64);
        impl HistoryApi for DailyFake {
            fn intraday(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: NaiveDate,
                _: NaiveDate,
            ) -> Result<Vec<Bar>, DataError> {
                Ok(Vec::new())
            }
            fn daily(
                &self,
                _: &str,
                _: &str,
                _: NaiveDate,
                _: NaiveDate,
            ) -> Result<Vec<Bar>, DataError> {
                Ok(vec![bar(self.0, 100.)])
            }
        }
        let mut updater = Updater {
            client: Box::new(DailyFake(ts)),
            catalog: Catalog::open(&db).unwrap(),
            store: Store::new(&root),
            max_retries: 0,
            margin_seconds: 0,
            reference_symbol_id: None,
            budget_cap: DEFAULT_BUDGET_CAP,
        };
        updater.update("5", Tf::D1, "EQUITY", d, d).unwrap();
        assert_eq!(
            updater.catalog.covered_dates("5", Tf::D1, d, d).unwrap(),
            vec![d]
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn progress_filter_handles_normal_and_short_sessions() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
        let open = d
            .and_hms_opt(9, 15, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        let end = d
            .and_hms_opt(15, 30, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        let normal = (0..375).map(|n| bar(open + n * 60, 1.0)).collect();
        assert_eq!(
            complete_intraday(normal, end, Tf::M1, &HashMap::new(), 0).len(),
            375
        );
        let short = d
            .and_hms_opt(15, 15, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        let ends = HashMap::from([(d.to_string(), short)]);
        let short_rows = (0..360).map(|n| bar(open + n * 60, 1.0)).collect();
        assert_eq!(
            complete_intraday(short_rows, short, Tf::M1, &ends, 0).len(),
            360
        );
    }
    #[test]
    fn atomic_store_and_catalog_reconcile() {
        let root = temp("atomic");
        let db = root.join("catalog.sqlite");
        let store = Store::new(&root);
        let ts = NaiveDate::from_ymd_opt(2026, 1, 31)
            .unwrap()
            .and_hms_opt(18, 29, 0)
            .unwrap()
            .and_utc()
            .timestamp();
        let parts = store
            .write_partitions("1", Tf::M1, &[bar(ts, 10.0), bar(ts + 60 * 24, 11.0)])
            .unwrap();
        assert_eq!(parts.len(), 2);
        {
            let mut cat = Catalog::open(&db).unwrap();
            cat.refresh_instruments(
                &[Scrip {
                    security_id: "1".into(),
                    symbol: "ABC".into(),
                    lot_size: 1,
                    tick_size_paise: 5,
                    instrument: "EQUITY".into(),
                }],
                0,
            )
            .unwrap();
            let sid = cat.series_for_import("1", Tf::M1).unwrap();
            for part in &parts {
                cat.record_partition(sid, &store, part).unwrap();
            }
            assert!(cat.has_coverage("1", Tf::M1, ts, ts).unwrap());
            store.reconcile(&mut cat).unwrap();
            std::fs::remove_file(&parts[0].path).unwrap();
            store.reconcile(&mut cat).unwrap();
            assert!(!cat.has_coverage("1", Tf::M1, ts, ts).unwrap());
        }
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn orphan_partition_is_reported_after_write_before_catalog_commit() {
        let root = temp("orphan");
        let db = root.join("catalog.sqlite");
        let store = Store::new(&root);
        let ts = NaiveDate::from_ymd_opt(2026, 4, 1)
            .unwrap()
            .and_hms_opt(9, 15, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        let parts = store
            .write_partitions("99", Tf::M1, &[bar(ts, 42.0)])
            .unwrap();
        let mut cat = Catalog::open(db).unwrap();
        store.reconcile(&mut cat).unwrap();
        store.reconcile(&mut cat).unwrap();
        assert_eq!(cat.quality("reconcile_orphan_file").unwrap(), 1);
        assert!(parts[0].path.exists());
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn coverage_details_joins_series_to_partition_rows() {
        let root = temp("coverage-details");
        let db = root.join("catalog.sqlite");
        let store = Store::new(&root);
        let ts = NaiveDate::from_ymd_opt(2026, 10, 28)
            .unwrap()
            .and_hms_opt(9, 15, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        let parts = store
            .write_partitions("2885", Tf::M1, &[bar(ts, 100.0), bar(ts + 60, 101.0)])
            .unwrap();
        let mut cat = Catalog::open(db).unwrap();
        cat.refresh_instruments(
            &[Scrip {
                security_id: "2885".into(),
                symbol: "RELIANCE".into(),
                lot_size: 1,
                tick_size_paise: 10,
                instrument: "EQUITY".into(),
            }],
            0,
        )
        .unwrap();
        let sid = cat.series_for_import("2885", Tf::M1).unwrap();
        for p in &parts {
            cat.record_partition(sid, &store, p).unwrap();
        }
        assert_eq!(
            cat.coverage_details().unwrap(),
            vec![("RELIANCE".into(), "1m".into(), 2)]
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn open_migrates_legacy_catalog_without_verify_and_sets_user_version() {
        let root = temp("migration");
        fs::create_dir_all(&root).unwrap();
        let db = root.join("catalog.sqlite");
        let legacy = Connection::open(&db).unwrap();
        legacy.execute_batch("CREATE TABLE instruments(security_id TEXT PRIMARY KEY,symbol TEXT NOT NULL UNIQUE,segment TEXT NOT NULL,lot_size INTEGER NOT NULL,tick_size_paise INTEGER NOT NULL,refreshed_at INTEGER NOT NULL); INSERT INTO instruments VALUES('1','ABC','NSE_EQ',1,5,0);").unwrap();
        drop(legacy);
        let cat = Catalog::open(&db).unwrap();
        assert_eq!(cat.instrument("1").unwrap(), "EQUITY");
        let version: i64 = cat
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 1);
        assert_eq!(cat.status_rows().unwrap().len(), 0);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn status_ranges_coalesce_touching_ist_dates_per_series() {
        let root = temp("status-ranges");
        let mut cat = Catalog::open(root.join("catalog.sqlite")).unwrap();
        cat.refresh_instruments(
            &[Scrip {
                security_id: "1".into(),
                symbol: "ABC".into(),
                lot_size: 1,
                tick_size_paise: 5,
                instrument: "EQUITY".into(),
            }],
            0,
        )
        .unwrap();
        let a = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let dates = [
            a,
            a + chrono::Days::new(1),
            a + chrono::Days::new(2),
            a + chrono::Days::new(3),
            a + chrono::Days::new(4),
            a + chrono::Days::new(7),
            a + chrono::Days::new(9),
            a + chrono::Days::new(11),
        ];
        for d in dates {
            cat.record_coverage(
                "1",
                Tf::M1,
                local_midnight(d),
                local_midnight(d + chrono::Days::new(1)) - 1,
                "data",
            )
            .unwrap();
        }
        cat.record_calendar(a + chrono::Days::new(8), false, "holiday")
            .unwrap();
        let rows = cat.status_rows().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].coverage,
            vec![
                CoverageRange {
                    from: a,
                    to: a + chrono::Days::new(9),
                    kind: "data".into()
                },
                CoverageRange {
                    from: a + chrono::Days::new(11),
                    to: a + chrono::Days::new(11),
                    kind: "data".into()
                }
            ]
        );
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn session_regime_change_detects_august_2026_once() {
        let mut shapes = BTreeMap::new();
        for n in 0..5 {
            let d = NaiveDate::from_ymd_opt(2026, 7, 27).unwrap() + chrono::Days::new(n);
            let end = d
                .and_hms_opt(15, 29, 0)
                .unwrap()
                .and_local_timezone(Kolkata)
                .single()
                .unwrap()
                .timestamp();
            shapes.insert(d, (0, end, 375));
        }
        let start = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
        for n in 0..12 {
            let d = start + chrono::Days::new(n);
            let end = d
                .and_hms_opt(15, 14, 0)
                .unwrap()
                .and_local_timezone(Kolkata)
                .single()
                .unwrap()
                .timestamp();
            shapes.insert(d, (0, end, 360));
        }
        let changes = session_regime_changes(&shapes);
        assert_eq!(changes, vec![start + chrono::Days::new(4)]);
    }
    #[test]
    fn repeated_identical_write_keeps_checksum_and_bytes() {
        let root = temp("idempotent");
        let store = Store::new(&root);
        let ts = NaiveDate::from_ymd_opt(2026, 2, 2)
            .unwrap()
            .and_hms_opt(9, 15, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        let b = bar(ts, 10.0);
        let a = store
            .write_partitions("x", Tf::M1, std::slice::from_ref(&b))
            .unwrap();
        let bytes = fs::read(&a[0].path).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let z = store.write_partitions("x", Tf::M1, &[b]).unwrap();
        assert_eq!(a[0].checksum, z[0].checksum);
        assert_eq!(bytes, fs::read(&z[0].path).unwrap());
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn month_partition_uses_ist_day() {
        let root = temp("ist");
        let store = Store::new(&root);
        let local = NaiveDate::from_ymd_opt(2026, 1, 31)
            .unwrap()
            .and_hms_opt(23, 59, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        let p = store
            .write_partitions("1", Tf::M1, &[bar(local, 10.0)])
            .unwrap();
        assert!(p[0].path.to_string_lossy().contains("2026-01"));
        let _ = fs::remove_dir_all(root);
    }

    fn scrip(id: &str, sym: &str) -> Scrip {
        Scrip {
            security_id: id.into(),
            symbol: sym.into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
        }
    }
    #[test]
    fn plan_requests_counts_89_day_chunks_and_single_daily() {
        let root = temp("plan-count");
        let mut cat = Catalog::open(root.join("catalog.sqlite")).unwrap();
        cat.refresh_instruments(&[scrip("1", "ABC")], 0).unwrap();
        let from = NaiveDate::from_ymd_opt(2021, 10, 31).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        assert_eq!(plan_requests(&cat, "1", Tf::M1, from, to).unwrap(), 21);
        assert_eq!(plan_requests(&cat, "1", Tf::D1, from, to).unwrap(), 1);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn dry_run_reports_per_tf_plan_and_sends_zero_requests() {
        let root = temp("bulk-dry");
        let db = root.join("catalog.sqlite");
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(&[scrip("10", "AAA"), scrip("2", "BBB")], 0)
            .unwrap();
        drop(cat);
        let from = NaiveDate::from_ymd_opt(2021, 10, 31).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let bulk = BulkDownloader {
            make_client: move || Box::new(CountingHistory { calls: c.clone() }) as Box<dyn HistoryApi>,
            catalog_path: db.clone(),
            store_root: root.clone(),
            config: BulkConfig::new(from, to, vec![Tf::D1, Tf::M1]),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let report = bulk.dry_run().unwrap();
        assert_eq!(report.symbols_total, 2);
        assert_eq!(
            report.planned,
            vec![("1d".to_string(), 2), ("1m".to_string(), 42)]
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn bulk_orders_priority_symbols_first_then_security_id() {
        let root = temp("bulk-order");
        let db = root.join("catalog.sqlite");
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(
            &[scrip("10", "AAA"), scrip("2", "BBB"), scrip("30", "CCC")],
            0,
        )
        .unwrap();
        drop(cat);
        let from = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let to = from + chrono::Days::new(1);
        let mut cfg = BulkConfig::new(from, to, vec![Tf::M1]);
        cfg.priority_symbols = vec!["CCC".into()];
        let bulk = BulkDownloader {
            make_client: || Box::new(CountingHistory {
                calls: Arc::new(AtomicUsize::new(0)),
            }) as Box<dyn HistoryApi>,
            catalog_path: db.clone(),
            store_root: root.clone(),
            config: cfg,
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let cat = Catalog::open(&db).unwrap();
        let order = bulk.order(&cat).unwrap();
        let symbols: Vec<_> = order.iter().map(|(_, s, _)| s.clone()).collect();
        // CCC (priority) first, then by numeric security id: 2=BBB, 10=AAA.
        assert_eq!(symbols, vec!["CCC", "BBB", "AAA"]);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn bulk_stops_at_soft_cap_and_budget_survives_restart() {
        let root = temp("bulk-budget");
        let db = root.join("catalog.sqlite");
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(&[scrip("1", "AAA"), scrip("2", "BBB")], 0)
            .unwrap();
        // Pre-seed today's budget so the cap is reached after the first symbol.
        let today = chrono::Utc::now().date_naive().to_string();
        cat.budget_add(&today, 1).unwrap();
        drop(cat);
        let from = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let to = from + chrono::Days::new(1);
        let mut cfg = BulkConfig::new(from, to, vec![Tf::D1]);
        cfg.budget_cap = 2; // room for exactly one more request
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let bulk = BulkDownloader {
            make_client: move || Box::new(FakeHistory {
                calls: c.clone(),
                empty_first: false,
            }) as Box<dyn HistoryApi>,
            catalog_path: db.clone(),
            store_root: root.clone(),
            config: cfg,
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let report = bulk.run(|_| {}).unwrap();
        assert!(report.budget_reached);
        // One request got through (budget 1 -> 2 == cap), the second symbol is skipped.
        let cat = Catalog::open(&db).unwrap();
        assert_eq!(cat.budget_used(&today).unwrap(), 2);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn bulk_resume_does_not_re_request_completed_ranges() {
        let root = temp("bulk-resume");
        let db = root.join("catalog.sqlite");
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(&[scrip("7", "AAA")], 0).unwrap();
        drop(cat);
        let from = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
        let to = from + chrono::Days::new(1);
        let calls = Arc::new(AtomicUsize::new(0));
        let make = {
            let calls = calls.clone();
            move || {
                Box::new(FakeHistory {
                    calls: calls.clone(),
                    empty_first: false,
                }) as Box<dyn HistoryApi>
            }
        };
        let mut cfg = BulkConfig::new(from, to, vec![Tf::M1]);
        cfg.reference_symbol_id = Some("7".into());
        let bulk = BulkDownloader {
            make_client: make,
            catalog_path: db.clone(),
            store_root: root.clone(),
            config: cfg,
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let first = bulk.run(|_| {}).unwrap();
        assert_eq!(first.symbols_completed, 1);
        let after_first = calls.load(Ordering::SeqCst);
        assert!(after_first >= 1);
        // Rerun: planner derives work from coverage, so nothing new is requested.
        let second = bulk.run(|_| {}).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), after_first);
        assert_eq!(second.requests_used, 0);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn progress_line_matches_spec_shape() {
        let p = Progress {
            tf: "1m",
            done: 842,
            total: 2365,
            requests: 17_904,
            budget_cap: 90_000,
            budget_used: 17_904,
            rate: 3.9,
            failures: 3,
        };
        let line = p.line();
        assert!(line.starts_with("1m  842/2365 symbols | requests 17,904/90,000 today |"));
        assert!(line.contains("3.9 req/s"));
        assert!(line.ends_with("failures 3"));
    }
    #[test]
    fn thousands_grouping_and_eta_formatting() {
        assert_eq!(group_thousands(90_000), "90,000");
        assert_eq!(group_thousands(17_904), "17,904");
        assert_eq!(group_thousands(5), "5");
        assert_eq!(format_eta(2 * 3600 + 41 * 60), "2h 41m");
        assert_eq!(format_eta(600), "10m");
        assert_eq!(format_eta(0), "0m");
    }
}
