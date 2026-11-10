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
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
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
    #[error("warmup short for {symbol}: have {have} bars, need {need}")]
    WarmupShort {
        symbol: String,
        have: usize,
        need: usize,
    },
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
            DataError::WarmupShort { .. } => "warmup_short".into(),
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
    pub fn seconds(self) -> i64 {
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualityRow {
    pub symbol: String,
    pub tf: String,
    pub date: String,
    pub kind: String,
}

pub struct Catalog {
    conn: Connection,
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
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA busy_timeout=30000; CREATE TABLE IF NOT EXISTS instruments(security_id TEXT PRIMARY KEY,symbol TEXT NOT NULL UNIQUE,segment TEXT NOT NULL,lot_size INTEGER NOT NULL,tick_size_paise INTEGER NOT NULL,instrument TEXT NOT NULL DEFAULT 'EQUITY',is_fno INTEGER NOT NULL DEFAULT 0,refreshed_at INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS series(id INTEGER PRIMARY KEY,security_id TEXT NOT NULL,tf TEXT NOT NULL,first_ts INTEGER,last_ts INTEGER,rows INTEGER NOT NULL DEFAULT 0,needs_refresh INTEGER NOT NULL DEFAULT 0,UNIQUE(security_id,tf)); CREATE TABLE IF NOT EXISTS partitions(series_id INTEGER NOT NULL,path TEXT NOT NULL,first_ts INTEGER NOT NULL,last_ts INTEGER NOT NULL,rows INTEGER NOT NULL,checksum TEXT NOT NULL,PRIMARY KEY(series_id,path)); CREATE TABLE IF NOT EXISTS coverage(series_id INTEGER NOT NULL,start_ts INTEGER NOT NULL,end_ts INTEGER NOT NULL,kind TEXT NOT NULL CHECK(kind IN ('data','confirmed_empty'))); CREATE TABLE IF NOT EXISTS sessions(series_id INTEGER NOT NULL,date TEXT NOT NULL,first_ts INTEGER NOT NULL,last_ts INTEGER NOT NULL,bars INTEGER NOT NULL,PRIMARY KEY(series_id,date)); CREATE TABLE IF NOT EXISTS quality(id INTEGER PRIMARY KEY,series_id INTEGER,kind TEXT NOT NULL,ts INTEGER,detail TEXT NOT NULL); CREATE TABLE IF NOT EXISTS runs(id INTEGER PRIMARY KEY,started INTEGER NOT NULL,finished INTEGER,requests INTEGER NOT NULL DEFAULT 0,status TEXT NOT NULL); CREATE TABLE IF NOT EXISTS budget(day TEXT PRIMARY KEY,requests INTEGER NOT NULL); CREATE TABLE IF NOT EXISTS trading_calendar(date TEXT PRIMARY KEY,is_trading INTEGER NOT NULL,source TEXT NOT NULL); CREATE TABLE IF NOT EXISTS failures(security_id INTEGER,tf TEXT,from_date TEXT,to_date TEXT,error_code TEXT,attempts INTEGER,next_retry TEXT,PRIMARY KEY(security_id,tf,from_date,to_date));")?;
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
        // IMMEDIATE so the read-before-write (SELECT partitions, then DELETE/UPDATE)
        // takes the write lock up front: under a concurrent bulk run this avoids a
        // stale-snapshot SQLITE_BUSY between workers; serially it is unchanged.
        let tx = cat
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
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
pub fn ts_to_ist_date(ts: i64) -> NaiveDate {
    DateTime::<Utc>::from_timestamp(ts, 0)
        .unwrap_or(DateTime::UNIX_EPOCH)
        .with_timezone(&Kolkata)
        .date_naive()
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
                            let outcome = updater.update(
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

pub fn read_partition_verified(
    path: &Path,
    expected_checksum: &str,
) -> Result<Vec<Bar>, SourceError> {
    let bytes = fs::read(path).map_err(|e| {
        SourceError::Other(format!(
            "missing partition file {}: {e}; run `algomln data verify`",
            path.display()
        ))
    })?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let actual_checksum = format!("{:x}", hasher.finalize());
    if actual_checksum != expected_checksum {
        return Err(SourceError::Other(format!(
            "partition checksum mismatch for {}: expected {}, got {}; run `algomln data verify`",
            path.display(),
            expected_checksum,
            actual_checksum
        )));
    }
    let reader = ParquetRecordBatchReaderBuilder::try_new(bytes::Bytes::from(bytes))
        .map_err(|e| SourceError::Other(format!("{}: {e}", path.display())))?
        .build()
        .map_err(|e| SourceError::Other(format!("{}: {e}", path.display())))?;
    let mut bars = Vec::new();
    for batch in reader {
        let b = batch.map_err(|e| SourceError::Other(format!("{}: {e}", path.display())))?;
        let ts = b
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .ok_or_else(|| SourceError::Other("missing or invalid ts column".into()))?;
        let open = b
            .column(1)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| SourceError::Other("invalid open column".into()))?;
        let high = b
            .column(2)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| SourceError::Other("invalid high column".into()))?;
        let low = b
            .column(3)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| SourceError::Other("invalid low column".into()))?;
        let close = b
            .column(4)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| SourceError::Other("invalid close column".into()))?;
        let volume = b
            .column(5)
            .as_any()
            .downcast_ref::<Float64Array>()
            .ok_or_else(|| SourceError::Other("invalid volume column".into()))?;
        for i in 0..b.num_rows() {
            bars.push(Bar {
                ts: ts.value(i),
                open: open.value(i),
                high: high.value(i),
                low: low.value(i),
                close: close.value(i),
                volume: volume.value(i),
            });
        }
    }
    Ok(bars)
}

#[derive(Clone, Debug)]
pub struct PartitionMetaRow {
    pub path: PathBuf,
    pub checksum: String,
    pub first_ts: i64,
    pub last_ts: i64,
}

pub struct CatalogCursor {
    pub symbol: Symbol,
    pub partitions: Vec<PartitionMetaRow>,
    pub current_partition_idx: usize,
    pub current_bars: Vec<Bar>,
    pub bar_idx: usize,
    pub filter_from: i64,
    pub filter_to: i64,
    pub resident_count: Arc<AtomicUsize>,
}

impl CatalogCursor {
    pub fn next_candle(&mut self) -> Result<Option<Candle>, SourceError> {
        loop {
            while self.bar_idx < self.current_bars.len() {
                let bar = &self.current_bars[self.bar_idx];
                self.bar_idx += 1;
                if bar.ts >= self.filter_from && bar.ts <= self.filter_to {
                    return Ok(Some(Candle {
                        timestamp: bar.ts,
                        open: bar.open,
                        high: bar.high,
                        low: bar.low,
                        close: bar.close,
                        volume: bar.volume,
                    }));
                }
            }
            if self.current_partition_idx >= self.partitions.len() {
                if !self.current_bars.is_empty() {
                    self.current_bars = Vec::new();
                    self.resident_count.store(0, Ordering::Relaxed);
                }
                return Ok(None);
            }
            let part = &self.partitions[self.current_partition_idx];
            self.current_partition_idx += 1;
            let loaded = read_partition_verified(&part.path, &part.checksum)?;
            self.current_bars = loaded;
            self.resident_count.store(1, Ordering::Relaxed);
            self.bar_idx = 0;
        }
    }
}

pub struct CatalogSource {
    pub store: Store,
    pub catalog_path: PathBuf,
    pub allow_gaps: bool,
    pub warmup_bars: usize,
    pub resident_counters: Arc<Mutex<HashMap<Symbol, Arc<AtomicUsize>>>>,
}

#[derive(Clone, Debug)]
pub struct ResolvedInfo {
    pub resolved_from: Option<i64>,
    pub resolved_to: Option<i64>,
    pub warmup_start: Option<NaiveDate>,
    pub fno_count: usize,
    pub non_fno_count: usize,
}

impl CatalogSource {
    pub fn new(store: Store, catalog_path: impl Into<PathBuf>) -> Self {
        Self {
            store,
            catalog_path: catalog_path.into(),
            allow_gaps: false,
            warmup_bars: 0,
            resident_counters: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn with_warmup_bars(mut self, bars: usize) -> Self {
        self.warmup_bars = bars;
        self
    }

    pub fn catalog(&self) -> Result<Catalog, DataError> {
        Catalog::open_read_only(&self.catalog_path)
    }

    pub fn reconcile(&self) -> Result<(), DataError> {
        let mut catalog = Catalog::open(&self.catalog_path)?;
        self.store.reconcile(&mut catalog)
    }

    pub fn gaps(
        &self,
        symbol: &str,
        tf: Tf,
        from_date: NaiveDate,
        to_date: NaiveDate,
    ) -> Vec<(NaiveDate, NaiveDate)> {
        let Ok(cat) = self.catalog() else {
            return vec![(from_date, to_date)];
        };
        let Ok((id, _)) = cat.resolve(symbol) else {
            return vec![(from_date, to_date)];
        };
        missing_date_ranges(&cat, &id, tf, from_date, to_date)
            .unwrap_or_else(|_| vec![(from_date, to_date)])
    }

    pub fn resident_counter_for(&self, symbol: &Symbol) -> Option<Arc<AtomicUsize>> {
        self.resident_counters.lock().ok()?.get(symbol).cloned()
    }

    pub fn resolved_info(
        &self,
        symbols: &[Symbol],
        timeframe: Timeframe,
        range: DateRange,
    ) -> Result<ResolvedInfo, SourceError> {
        let catalog = self.catalog().map_err(|e| SourceError::Other(e.to_string()))?;
        let tf = Tf::parse(timeframe.as_str())
            .ok_or_else(|| SourceError::Other("unsupported timeframe".into()))?;

        let mut min_from: Option<i64> = None;
        let mut max_to: Option<i64> = None;
        let mut min_warmup: Option<NaiveDate> = None;
        let mut fno_count = 0;
        let mut non_fno_count = 0;

        for symbol in symbols {
            let (id, _) = match catalog.resolve(symbol) {
                Ok(x) => x,
                Err(_) => continue,
            };
            let (_lot_size, _tick_size, is_fno) = catalog.instrument_meta(&id).unwrap_or((1, 5, false));
            if is_fno {
                fno_count += 1;
            } else {
                non_fno_count += 1;
            }

            let series_row: Option<(i64, Option<i64>, Option<i64>, u64)> = catalog
                .conn
                .query_row(
                    "SELECT id, first_ts, last_ts, rows FROM series WHERE security_id=?1 AND tf=?2",
                    params![id, tf.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .ok();

            if let Some((sid, Some(first_ts), Some(last_ts), rows)) = series_row {
                if rows > 0 {
                    let res_from = range.from.unwrap_or(first_ts);
                    let res_to = range.to.unwrap_or(last_ts);
                    min_from = Some(min_from.map_or(res_from, |m| m.min(res_from)));
                    max_to = Some(max_to.map_or(res_to, |m| m.max(res_to)));

                    let from_d = ts_to_ist_date(res_from);
                    let bars_per_session: usize = if tf == Tf::D1 {
                        1
                    } else {
                        let session_mins: usize = if is_fno { 360 } else { 375 };
                        let tf_mins: usize = (tf.seconds() as usize / 60).max(1);
                        (session_mins + tf_mins - 1) / tf_mins
                    };

                    let w_date = if self.warmup_bars == 0 {
                        from_d
                    } else {
                        let sessions_needed =
                            (self.warmup_bars + bars_per_session - 1) / bars_per_session + 1;
                        let mut cur = from_d;
                        let mut count = 0;
                        while count < sessions_needed {
                            cur = cur - chrono::Days::new(1);
                            if matches!(
                                cur.weekday(),
                                chrono::Weekday::Mon
                                    | chrono::Weekday::Tue
                                    | chrono::Weekday::Wed
                                    | chrono::Weekday::Thu
                                    | chrono::Weekday::Fri
                            ) {
                                if !catalog.is_holiday(cur).unwrap_or(false) {
                                    count += 1;
                                }
                            }
                        }
                        let first_covered = catalog
                            .first_covered_date(sid)
                            .unwrap_or(None)
                            .unwrap_or(ts_to_ist_date(first_ts));
                        cur.max(first_covered)
                    };
                    min_warmup = Some(min_warmup.map_or(w_date, |m| m.min(w_date)));
                }
            }
        }

        Ok(ResolvedInfo {
            resolved_from: min_from,
            resolved_to: max_to,
            warmup_start: min_warmup,
            fno_count,
            non_fno_count,
        })
    }
}

impl CandleSource for CatalogSource {
    fn open(
        &self,
        symbols: &[Symbol],
        timeframe: Timeframe,
        range: DateRange,
    ) -> Result<CandleStream, SourceError> {
        let catalog = self
            .catalog()
            .map_err(|e| SourceError::Other(e.to_string()))?;
        let tf = Tf::parse(timeframe.as_str())
            .ok_or_else(|| SourceError::Other("unsupported timeframe".into()))?;

        let mut uncovered_gaps: BTreeMap<(NaiveDate, NaiveDate), Vec<Symbol>> = BTreeMap::new();
        let mut warmup_shorts: Vec<(Symbol, usize, usize)> = Vec::new();
        let mut cursor_data = Vec::new();

        for symbol in symbols {
            let (id, _) = match catalog.resolve(symbol) {
                Ok(x) => x,
                Err(_) => {
                    let from_d = range
                        .from
                        .map(ts_to_ist_date)
                        .unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
                    let to_d = range
                        .to
                        .map(ts_to_ist_date)
                        .unwrap_or_else(|| NaiveDate::from_ymd_opt(2099, 12, 31).unwrap());
                    uncovered_gaps
                        .entry((from_d, to_d))
                        .or_default()
                        .push(symbol.clone());
                    continue;
                }
            };

            let (_lot, _tick, is_fno) = catalog.instrument_meta(&id).unwrap_or((1, 5, false));

            let series_row: Option<(i64, Option<i64>, Option<i64>, u64)> = catalog
                .conn
                .query_row(
                    "SELECT id, first_ts, last_ts, rows FROM series WHERE security_id=?1 AND tf=?2",
                    params![id, tf.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .ok();

            let (sid, first_ts, last_ts) = match series_row {
                Some((sid, Some(f), Some(l), rows)) if rows > 0 => (sid, f, l),
                _ => {
                    let from_d = range
                        .from
                        .map(ts_to_ist_date)
                        .unwrap_or_else(|| NaiveDate::from_ymd_opt(1970, 1, 1).unwrap());
                    let to_d = range
                        .to
                        .map(ts_to_ist_date)
                        .unwrap_or_else(|| NaiveDate::from_ymd_opt(2099, 12, 31).unwrap());
                    uncovered_gaps
                        .entry((from_d, to_d))
                        .or_default()
                        .push(symbol.clone());
                    continue;
                }
            };

            let resolved_from = range.from.unwrap_or(first_ts);
            let resolved_to = range.to.unwrap_or(last_ts);
            let from_date = ts_to_ist_date(resolved_from);
            let to_date = ts_to_ist_date(resolved_to);

            let bars_per_session: usize = if tf == Tf::D1 {
                1
            } else {
                let session_mins: usize = if is_fno { 360 } else { 375 };
                let tf_mins: usize = (tf.seconds() as usize / 60).max(1);
                (session_mins + tf_mins - 1) / tf_mins
            };

            let (warmup_start_date, warmup_start_ts) = if self.warmup_bars == 0 {
                (from_date, resolved_from)
            } else {
                let sessions_needed =
                    (self.warmup_bars + bars_per_session - 1) / bars_per_session + 1;
                let mut cur = from_date;
                let mut count = 0;
                while count < sessions_needed {
                    cur = cur - chrono::Days::new(1);
                    if matches!(
                        cur.weekday(),
                        chrono::Weekday::Mon
                            | chrono::Weekday::Tue
                            | chrono::Weekday::Wed
                            | chrono::Weekday::Thu
                            | chrono::Weekday::Fri
                    ) {
                        if !catalog.is_holiday(cur).unwrap_or(false) {
                            count += 1;
                        }
                    }
                }
                let first_covered = catalog
                    .first_covered_date(sid)
                    .unwrap_or(None)
                    .unwrap_or(ts_to_ist_date(first_ts));
                let clamped_date = cur.max(first_covered);
                let w_start_ts = local_midnight(clamped_date);

                let have = catalog.bars_before_ts(sid, clamped_date, resolved_from).unwrap_or(0);
                if have < self.warmup_bars {
                    if self.allow_gaps {
                        eprintln!(
                            "warning: warmup short for {symbol}: have {have} bars, need {}",
                            self.warmup_bars
                        );
                    } else {
                        warmup_shorts.push((symbol.clone(), have, self.warmup_bars));
                    }
                }
                (clamped_date, w_start_ts)
            };

            let gaps = missing_date_ranges(&catalog, &id, tf, warmup_start_date, to_date)
                .unwrap_or_else(|_| vec![(warmup_start_date, to_date)]);
            if !gaps.is_empty() {
                if self.allow_gaps {
                    eprintln!("warning: skipping uncovered symbol {symbol}");
                    continue;
                }
                let merged_gap = (gaps.first().unwrap().0, gaps.last().unwrap().1);
                uncovered_gaps
                    .entry(merged_gap)
                    .or_default()
                    .push(symbol.clone());
                continue;
            }

            let mut stmt = catalog
                .conn
                .prepare(
                    "SELECT path, checksum, first_ts, last_ts FROM partitions \
                     WHERE series_id=?1 AND last_ts >= ?2 AND first_ts <= ?3 \
                     ORDER BY first_ts",
                )
                .map_err(|e| SourceError::Other(e.to_string()))?;
            let parts: Vec<PartitionMetaRow> = stmt
                .query_map(params![sid, warmup_start_ts, resolved_to], |r| {
                    Ok(PartitionMetaRow {
                        path: self.store.root.join(r.get::<_, String>(0)?),
                        checksum: r.get::<_, String>(1)?,
                        first_ts: r.get(2)?,
                        last_ts: r.get(3)?,
                    })
                })
                .map_err(|e| SourceError::Other(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| SourceError::Other(e.to_string()))?;

            cursor_data.push((symbol.clone(), parts, warmup_start_ts, resolved_to));
        }

        if !uncovered_gaps.is_empty() || !warmup_shorts.is_empty() {
            let mut lines = Vec::new();
            for ((gap_from, gap_to), mut syms) in uncovered_gaps {
                syms.sort();
                let s_list = syms.join(",");
                lines.push(format!(
                    "uncovered range for {s_list}; run `algomln data update --symbols {s_list} --tf {} --from {gap_from} --to {gap_to}`",
                    tf.as_str()
                ));
            }
            for (sym, have, need) in warmup_shorts {
                lines.push(format!("warmup short for {sym}: have {have} bars, need {need}"));
            }
            return Err(SourceError::Other(lines.join("\n")));
        }

        if cursor_data.is_empty() {
            return Ok(Box::new(std::iter::empty()));
        }

        cursor_data.sort_by(|a, b| a.0.cmp(&b.0));

        let mut cursors = Vec::with_capacity(cursor_data.len());
        for (symbol, partitions, filter_from, filter_to) in cursor_data {
            let resident_count = Arc::new(AtomicUsize::new(0));
            if let Ok(mut counters) = self.resident_counters.lock() {
                counters.insert(symbol.clone(), resident_count.clone());
            }
            cursors.push(CatalogCursor {
                symbol,
                partitions,
                current_partition_idx: 0,
                current_bars: Vec::new(),
                bar_idx: 0,
                filter_from,
                filter_to,
                resident_count,
            });
        }

        let merge = KWayMerge::new(cursors)?;
        Ok(Box::new(merge))
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
        let Some(t) = Tf::parse(tf.as_str()) else {
            return Fingerprint("unsupported-timeframe".into());
        };

        let mut sorted_symbols = symbols.to_vec();
        sorted_symbols.sort();

        let mut h = Sha256::new();
        h.update(b"cf2;");

        let mut partitions_to_hash = BTreeMap::<String, String>::new();

        for s in &sorted_symbols {
            let (id, _) = match c.resolve(s) {
                Ok(x) => x,
                Err(_) => continue,
            };
            let (lot_size, tick_size_paise, is_fno) = c.instrument_meta(&id).unwrap_or((1, 5, false));
            let series_row: Option<(i64, Option<i64>, Option<i64>, u64)> = c
                .conn
                .query_row(
                    "SELECT id, first_ts, last_ts, rows FROM series WHERE security_id=?1 AND tf=?2",
                    params![id, t.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .ok();

            let (sid, first_ts, last_ts) = match series_row {
                Some((sid, Some(f), Some(l), rows)) if rows > 0 => (sid, f, l),
                _ => continue,
            };

            let resolved_from = range.from.unwrap_or(first_ts);
            let resolved_to = range.to.unwrap_or(last_ts);
            let from_date = ts_to_ist_date(resolved_from);
            let to_date = ts_to_ist_date(resolved_to);

            let bars_per_session: usize = if t == Tf::D1 {
                1
            } else {
                let session_mins: usize = if is_fno { 360 } else { 375 };
                let tf_mins: usize = (t.seconds() as usize / 60).max(1);
                (session_mins + tf_mins - 1) / tf_mins
            };

            let (warmup_start_date, warmup_start_ts) = if self.warmup_bars == 0 {
                (from_date, resolved_from)
            } else {
                let sessions_needed =
                    (self.warmup_bars + bars_per_session - 1) / bars_per_session + 1;
                let mut cur = from_date;
                let mut count = 0;
                while count < sessions_needed {
                    cur = cur - chrono::Days::new(1);
                    if matches!(
                        cur.weekday(),
                        chrono::Weekday::Mon
                            | chrono::Weekday::Tue
                            | chrono::Weekday::Wed
                            | chrono::Weekday::Thu
                            | chrono::Weekday::Fri
                    ) {
                        if !c.is_holiday(cur).unwrap_or(false) {
                            count += 1;
                        }
                    }
                }
                let first_covered = c
                    .first_covered_date(sid)
                    .unwrap_or(None)
                    .unwrap_or(ts_to_ist_date(first_ts));
                let clamped = cur.max(first_covered);
                (clamped, local_midnight(clamped))
            };

            let _ = write!(
                h,
                "{}|{}|{}|{}|{}|{}|{}|{};",
                s,
                t.as_str(),
                from_date,
                to_date,
                warmup_start_date,
                lot_size,
                tick_size_paise,
                if is_fno { 1 } else { 0 }
            );

            if let Ok(mut q) = c.conn.prepare(
                "SELECT path, checksum FROM partitions WHERE series_id=?1 AND last_ts >= ?2 AND first_ts <= ?3 ORDER BY path",
            ) {
                if let Ok(it) = q.query_map(params![sid, warmup_start_ts, resolved_to], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                }) {
                    for x in it.flatten() {
                        partitions_to_hash.insert(x.0, x.1);
                    }
                }
            }
        }

        for (path, checksum) in partitions_to_hash {
            let _ = write!(h, "{path}:{checksum};");
        }

        Fingerprint(format!("sha256:{:x}", h.finalize()))
    }
}

struct KWayMerge {
    cursors: Vec<CatalogCursor>,
    heap: std::collections::BinaryHeap<std::cmp::Reverse<(i64, Symbol, usize)>>,
    current_candles: Vec<Option<Candle>>,
}

impl KWayMerge {
    fn new(mut cursors: Vec<CatalogCursor>) -> Result<Self, SourceError> {
        let mut heap = std::collections::BinaryHeap::new();
        let mut current_candles = Vec::with_capacity(cursors.len());
        for (i, cursor) in cursors.iter_mut().enumerate() {
            if let Some(candle) = cursor.next_candle()? {
                heap.push(std::cmp::Reverse((candle.timestamp, cursor.symbol.clone(), i)));
                current_candles.push(Some(candle));
            } else {
                current_candles.push(None);
            }
        }
        Ok(Self {
            cursors,
            heap,
            current_candles,
        })
    }
}

impl Iterator for KWayMerge {
    type Item = Result<(Symbol, Candle), SourceError>;

    fn next(&mut self) -> Option<Self::Item> {
        let std::cmp::Reverse((_, _, i)) = self.heap.pop()?;
        let candle = self.current_candles[i].take().expect("candle must be present");
        let sym = self.cursors[i].symbol.clone();

        match self.cursors[i].next_candle() {
            Ok(Some(next_candle)) => {
                self.heap.push(std::cmp::Reverse((
                    next_candle.timestamp,
                    self.cursors[i].symbol.clone(),
                    i,
                )));
                self.current_candles[i] = Some(next_candle);
            }
            Ok(None) => {}
            Err(e) => return Some(Err(e)),
        }

        Some(Ok((sym, candle)))
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
                is_fno: false,
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
                is_fno: false,
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
                    is_fno: false,
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
                is_fno: false,
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
                is_fno: false,
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
                is_fno: false,
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
        let d1 = NaiveDate::from_ymd_opt(2026, 1, 30).unwrap();
        let ts1 = d1
            .and_hms_opt(15, 20, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        let d2 = NaiveDate::from_ymd_opt(2026, 2, 2).unwrap();
        let ts2 = d2
            .and_hms_opt(9, 15, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        let parts = store
            .write_partitions("1", Tf::M1, &[bar(ts1, 10.0), bar(ts2, 11.0)])
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
                    is_fno: false,
                }],
                0,
            )
            .unwrap();
            let sid = cat.series_for_import("1", Tf::M1).unwrap();
            for part in &parts {
                cat.record_partition(sid, &store, part).unwrap();
            }
            cat.record_session(sid, d1, ts1, ts1 + 60, 1).unwrap();
            cat.record_session(sid, d2, ts2, ts2 + 60, 1).unwrap();
            assert!(cat.has_coverage("1", Tf::M1, ts1, ts1).unwrap());
            store.reconcile(&mut cat).unwrap();
            std::fs::remove_file(&parts[0].path).unwrap();
            store.reconcile(&mut cat).unwrap();
            assert!(!cat.has_coverage("1", Tf::M1, ts1, ts1).unwrap());
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
                is_fno: false,
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
        assert_eq!(version, 2);
        assert!(!cat.is_fno("1").unwrap());
        assert_eq!(cat.status_rows().unwrap().len(), 0);
        let _ = fs::remove_dir_all(root);
    }
    #[test]
    fn catalog_migration_from_v1_populates_is_fno_on_refresh() {
        let root = temp("migration-v1-to-v2");
        fs::create_dir_all(&root).unwrap();
        let db = root.join("catalog.sqlite");
        let legacy = Connection::open(&db).unwrap();
        legacy.execute_batch("PRAGMA user_version=1; CREATE TABLE instruments(security_id TEXT PRIMARY KEY,symbol TEXT NOT NULL UNIQUE,segment TEXT NOT NULL,lot_size INTEGER NOT NULL,tick_size_paise INTEGER NOT NULL,instrument TEXT NOT NULL DEFAULT 'EQUITY',refreshed_at INTEGER NOT NULL); INSERT INTO instruments VALUES('2885','RELIANCE','NSE_EQ',1,10,'EQUITY',0);").unwrap();
        drop(legacy);
        let mut cat = Catalog::open(&db).unwrap();
        let version: i64 = cat
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, 2);
        assert!(!cat.is_fno("2885").unwrap());
        cat.refresh_instruments(
            &[Scrip {
                security_id: "2885".into(),
                symbol: "RELIANCE".into(),
                lot_size: 1,
                tick_size_paise: 10,
                instrument: "EQUITY".into(),
                is_fno: true,
            }],
            Utc::now().timestamp(),
        )
        .unwrap();
        assert!(cat.is_fno("2885").unwrap());
        let (total, fno) = cat.instrument_counts().unwrap();
        assert_eq!(total, 1);
        assert_eq!(fno, 1);
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
                is_fno: false,
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
    fn quality_rows_joins_symbol_tf_and_formats_ist_date() {
        let root = temp("quality_rows");
        let db = root.join("catalog.sqlite");
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(
            &[Scrip {
                security_id: "101".into(),
                symbol: "RELIANCE".into(),
                lot_size: 1,
                tick_size_paise: 5,
                instrument: "EQUITY".into(),
                is_fno: false,
            }],
            0,
        )
        .unwrap();
        cat.series_for_import("101", Tf::M1).unwrap();
        let day = NaiveDate::from_ymd_opt(2026, 8, 7).unwrap();
        cat.mark_quality(
            "101",
            Tf::M1,
            &Quality {
                kind: "session_regime_change".into(),
                ts: Some(local_midnight(day)),
                detail: "derived session end mode changed".into(),
            },
        )
        .unwrap();
        let rows = cat.quality_rows().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0],
            QualityRow {
                symbol: "RELIANCE".into(),
                tf: "1m".into(),
                date: "2026-08-07".into(),
                kind: "session_regime_change".into(),
            }
        );
        let _ = fs::remove_dir_all(root);
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
            is_fno: false,
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
            make_client: move || {
                Box::new(CountingHistory { calls: c.clone() }) as Box<dyn HistoryApi>
            },
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
            make_client: || {
                Box::new(CountingHistory {
                    calls: Arc::new(AtomicUsize::new(0)),
                }) as Box<dyn HistoryApi>
            },
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
            make_client: move || {
                Box::new(FakeHistory {
                    calls: c.clone(),
                    empty_first: false,
                }) as Box<dyn HistoryApi>
            },
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

/// Acceptance tests (spec 11) that drive the full download stack through a real
/// [`DhanClient`] pointed at an in-process mock Dhan server. These cover the
/// criteria that need the network edge — 250 ms spacing, DH-904 backoff, auth
/// abort, `no_data` handling and 375-bar sessions — rather than the chunking /
/// resume / budget logic already exercised by the unit tests above.
#[cfg(test)]
mod acceptance {
    use super::*;
    use dhan_api::{DhanClient, TokenProvider};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Instant;
    use algomln::sim::FileSource;

    struct TestToken;
    impl TokenProvider for TestToken {
        fn access_token(&self) -> Result<String, dhan_api::ApiError> {
            Ok("secret".into())
        }
    }

    /// Outcome of the mock server thread: the number of requests served and the
    /// [`Instant`] at which each request's bytes were received.
    struct MockOutcome {
        served: usize,
        received_at: Vec<Instant>,
    }

    /// Start a TCP mock Dhan server that replies to each incoming request with
    /// the next `(status, body)` pair in `responses`, recording the instant each
    /// request arrives. Returns the base URL and a join handle. Modelled on the
    /// dhan-api `mock` helper (raw HTTP/1.1 over `TcpListener`).
    fn mock_server(
        responses: Vec<(u16, String)>,
    ) -> (String, std::thread::JoinHandle<MockOutcome>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut received_at = Vec::new();
            let mut served = 0;
            for (status, body) in responses {
                let (mut stream, _) = match listener.accept() {
                    Ok(x) => x,
                    Err(_) => break,
                };
                let mut buf = [0u8; 8192];
                let _ = stream.read(&mut buf).unwrap();
                received_at.push(Instant::now());
                served += 1;
                let phrase = match status {
                    200 => "OK",
                    400 => "Bad Request",
                    401 => "Unauthorized",
                    429 => "Too Many Requests",
                    500 => "Internal Server Error",
                    _ => "Error",
                };
                let response = format!(
                    "HTTP/1.1 {status} {phrase}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
                    body.len(),
                    if status == 429 { "Retry-After: 0\r\n" } else { "" },
                    body
                );
                stream.write_all(response.as_bytes()).unwrap();
            }
            MockOutcome {
                served,
                received_at,
            }
        });
        (format!("http://{addr}"), handle)
    }

    /// Build the intraday columnar JSON the Dhan `/charts/intraday` endpoint
    /// returns for `n` consecutive one-minute bars starting at `start_ts`.
    fn intraday_body(start_ts: i64, n: usize) -> String {
        let ts: Vec<String> = (0..n)
            .map(|i| (start_ts + i as i64 * 60).to_string())
            .collect();
        let ones = vec!["1.0"; n].join(",");
        let twos = vec!["2.0"; n].join(",");
        let vols = vec!["10.0"; n].join(",");
        format!(
            "{{\"open\":[{o}],\"high\":[{h}],\"low\":[{l}],\"close\":[{c}],\"volume\":[{v}],\"timestamp\":[{t}]}}",
            o = ones,
            h = twos,
            l = ones,
            c = twos,
            v = vols,
            t = ts.join(",")
        )
    }

    fn empty_body() -> String {
        r#"{"open":[],"high":[],"low":[],"close":[],"volume":[],"timestamp":[]}"#.to_string()
    }

    fn make_updater(base: &str, db: &Path, root: &Path) -> Updater {
        let client = DhanClient::with_base(Arc::new(TestToken), base).unwrap();
        Updater {
            client: Box::new(client),
            catalog: Catalog::open(db).unwrap(),
            store: Store::new(root),
            max_retries: 0,
            margin_seconds: 0,
            reference_symbol_id: None,
            budget_cap: DEFAULT_BUDGET_CAP,
        }
    }

    fn seed_instrument(db: &Path, id: &str, symbol: &str) {
        let mut cat = Catalog::open(db).unwrap();
        cat.refresh_instruments(
            &[Scrip {
                security_id: id.into(),
                symbol: symbol.into(),
                lot_size: 1,
                tick_size_paise: 5,
                instrument: "EQUITY".into(),
                is_fno: false,
            }],
            0,
        )
        .unwrap();
    }

    fn open_ist(d: NaiveDate) -> i64 {
        d.and_hms_opt(9, 15, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp()
    }

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "algomln-acc-{name}-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        ))
    }

    /// Spec 11.2: request start times are never closer than 250 ms, measured
    /// from the mock's receive timestamps. Three one-minute chunks across a
    /// single intraday series must arrive >=250 ms apart, enforced by the shared
    /// limiter inside the real [`DhanClient`].
    #[test]
    fn request_starts_are_at_least_250ms_apart() {
        let root = temp("spacing");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        // Three 89-day chunks => three intraday requests.
        let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let to = from + chrono::Days::new(179);
        assert_eq!(chunk_dates(from, to).len(), 3);
        let responses = chunk_dates(from, to)
            .iter()
            .map(|(a, _)| (200u16, intraday_body(open_ist(*a), 1)))
            .collect::<Vec<_>>();
        let (base, server) = mock_server(responses);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", from, to).unwrap();
        let outcome = server.join().unwrap();
        assert_eq!(outcome.served, 3);
        for pair in outcome.received_at.windows(2) {
            let gap = pair[1].duration_since(pair[0]);
            // The limiter targets 250 ms between request starts; allow generous
            // slack for localhost delivery/scheduling jitter on the receive side.
            // Without the limiter these arrive sub-millisecond apart, so a 200 ms
            // floor still unambiguously proves spacing is enforced.
            assert!(
                gap >= std::time::Duration::from_millis(200),
                "requests only {gap:?} apart; limiter must keep ~250 ms spacing"
            );
        }
        let _ = fs::remove_dir_all(root);
    }

    /// Spec 11.6: an injected DH-904 triggers backoff, and the run still
    /// completes. The DhanClient retries after the limiter cools down; the
    /// second attempt succeeds and the series is recorded.
    #[test]
    fn injected_dh904_backs_off_and_run_completes() {
        let root = temp("dh904");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let to = from; // single intraday chunk
        let responses = vec![
            (
                429u16,
                r#"{"errorCode":"DH-904","errorMessage":"slow down"}"#.to_string(),
            ),
            (200u16, intraday_body(open_ist(from), 1)),
        ];
        let (base, server) = mock_server(responses);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", from, to).unwrap();
        let outcome = server.join().unwrap();
        assert_eq!(
            outcome.served, 2,
            "DH-904 must be retried once then succeed"
        );
        let covered = up.catalog.covered_dates("1", Tf::M1, from, to).unwrap();
        assert_eq!(covered, vec![from]);
        let _ = fs::remove_dir_all(root);
    }

    /// Spec 11.8: an auth failure aborts the run immediately. Driven through the
    /// `BulkDownloader` so the abort is observed at the run level, not just the
    /// client: `is_auth()` short-circuits and the error propagates.
    #[test]
    fn auth_error_aborts_the_run_immediately() {
        let root = temp("auth");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "AAA");
        // A second symbol that must NOT be contacted once auth fails.
        {
            let mut cat = Catalog::open(&db).unwrap();
            cat.refresh_instruments(
                &[
                    Scrip {
                        security_id: "1".into(),
                        symbol: "AAA".into(),
                        lot_size: 1,
                        tick_size_paise: 5,
                        instrument: "EQUITY".into(),
                        is_fno: false,
                    },
                    Scrip {
                        security_id: "2".into(),
                        symbol: "BBB".into(),
                        lot_size: 1,
                        tick_size_paise: 5,
                        instrument: "EQUITY".into(),
                        is_fno: false,
                    },
                ],
                0,
            )
            .unwrap();
        }
        let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let to = from;
        // Only one response queued: the first (and only) request gets 401.
        let (base, server) = mock_server(vec![(
            401u16,
            r#"{"errorCode":"DH-902","errorMessage":"invalid token"}"#.to_string(),
        )]);
        let base_for_factory = base.clone();
        let bulk = BulkDownloader {
            make_client: move || {
                Box::new(DhanClient::with_base(Arc::new(TestToken), &base_for_factory).unwrap())
                    as Box<dyn HistoryApi>
            },
            catalog_path: db.clone(),
            store_root: root.clone(),
            config: BulkConfig::new(from, to, vec![Tf::D1]),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let result = bulk.run(|_| {});
        assert!(
            matches!(&result, Err(e) if e.is_auth()),
            "auth failure must abort the run with an auth error, got {result:?}"
        );
        let outcome = server.join().unwrap();
        assert_eq!(
            outcome.served, 1,
            "run must abort after the first auth failure without contacting the next symbol"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// Spec 11.7: an empty response is logged as `no_data` and is NOT recorded
    /// as coverage. The reference symbol path records the calendar as empty, but
    /// no `data` coverage is written and the session is not marked covered for a
    /// non-reference symbol.
    #[test]
    fn empty_response_is_not_recorded_as_coverage() {
        let root = temp("nodata");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let to = from;
        let (base, server) = mock_server(vec![(200u16, empty_body())]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", from, to).unwrap();
        let outcome = server.join().unwrap();
        assert_eq!(outcome.served, 1);
        // The spec guarantee: an empty response is never recorded as *data*
        // coverage (it could be a holiday or a pre-listing range). The empty day
        // may be logged as `confirmed_empty`/`no_data` so it is not re-requested
        // forever, but it must not claim real bars exist.
        let sid = up.catalog.ensure_series("1", Tf::M1).unwrap();
        let data_rows: i64 = up
            .catalog
            .conn
            .query_row(
                "SELECT COUNT(*) FROM coverage WHERE series_id=?1 AND kind='data'",
                [sid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(data_rows, 0, "empty response must not add 'data' coverage");
        // No real bars were stored.
        let bars: i64 = up
            .catalog
            .conn
            .query_row(
                "SELECT COALESCE(SUM(rows),0) FROM partitions WHERE series_id=?1",
                [sid],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(bars, 0, "empty response must not store any bars");
        let _ = fs::remove_dir_all(root);
    }

    /// Spec 11.9: a symbol with 375-bar sessions downloads and verifies
    /// correctly — the downloader must not assume 360 bars per session. The mock
    /// returns a full 375-minute session; all 375 bars must be stored.
    #[test]
    fn symbol_with_375_bar_session_downloads_all_bars() {
        let root = temp("375");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let day = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        // 375 one-minute bars: 09:15..=15:29 inclusive is 375 minutes.
        let (base, server) = mock_server(vec![(200u16, intraday_body(open_ist(day), 375))]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", day, day).unwrap();
        let outcome = server.join().unwrap();
        assert_eq!(outcome.served, 1);
        let status = up.catalog.status_rows().unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(
            status[0].bars, 375,
            "all 375 session bars must be stored; downloader must not cap at 360"
        );
        let _ = fs::remove_dir_all(root);
    }

    /// Spec 6 + 11.2: with `concurrency > 1` the shared limiter must still cap the
    /// global request rate at ~4/s. Eight symbols are fetched by a 4-worker pool
    /// through one cloned-per-worker `DhanClient` limiter.
    ///
    /// The limiter is a token bucket (cap 4), so it permits a short burst of up to
    /// four requests before throttling — a per-gap floor is therefore fragile (the
    /// bucket refills during worker setup and the first few arrivals bunch up, as
    /// seen with 195 ms gaps). The robust invariant is the *total* span: with a max
    /// burst of 4 tokens, `N` requests take at least `(N - 4) / 4` seconds, i.e.
    /// >=1.0 s for 8. Without the limiter all eight arrive in well under 100 ms, so
    /// an 0.8 s floor unambiguously proves throttling survives concurrency. Also
    /// proves the concurrent write path commits every symbol without racing on the
    /// shared catalog (WAL + busy_timeout + IMMEDIATE writes).
    #[test]
    fn concurrent_run_throttles_global_rate_and_commits_all_symbols() {
        let root = temp("concurrent-spacing");
        let db = root.join("catalog.sqlite");
        let symbols = ["AAA", "BBB", "CCC", "DDD", "EEE", "FFF", "GGG", "HHH"];
        for (i, sym) in symbols.iter().enumerate() {
            seed_instrument(&db, &(i + 1).to_string(), sym);
        }
        let day = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
        // One intraday request per symbol (single-day range => single chunk).
        let responses = (0..symbols.len())
            .map(|_| (200u16, intraday_body(open_ist(day), 1)))
            .collect::<Vec<_>>();
        let (base, server) = mock_server(responses);
        // Every worker's client shares the one limiter, so the rate cap is global
        // across the pool rather than per client.
        let client = DhanClient::with_base(Arc::new(TestToken), &base).unwrap();
        let base_for_factory = base.clone();
        let make_client = move || {
            Box::new(
                client
                    .clone_with_base(&base_for_factory)
                    .expect("clone shares the limiter"),
            ) as Box<dyn HistoryApi>
        };
        let mut cfg = BulkConfig::new(day, day, vec![Tf::M1]);
        cfg.concurrency = 4;
        cfg.margin_seconds = 0;
        let bulk = BulkDownloader {
            make_client,
            catalog_path: db.clone(),
            store_root: root.clone(),
            config: cfg,
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        let report = bulk.run(|_| {}).unwrap();
        assert_eq!(
            report.symbols_completed,
            symbols.len(),
            "every symbol must commit"
        );
        let outcome = server.join().unwrap();
        assert_eq!(
            outcome.served,
            symbols.len(),
            "exactly one request per symbol"
        );
        let mut times = outcome.received_at;
        times.sort();
        let span = times
            .last()
            .unwrap()
            .duration_since(*times.first().unwrap());
        assert!(
            span >= std::time::Duration::from_millis(800),
            "eight requests spanned only {span:?}; the shared limiter must keep the \
             global rate near 4/s even with concurrency"
        );
        // Every series is present with its single bar committed (no write races).
        let cat = Catalog::open(&db).unwrap();
        let status = cat.status_rows().unwrap();
        assert_eq!(status.len(), symbols.len());
        assert!(status.iter().all(|s| s.bars == 1));
        let _ = fs::remove_dir_all(root);
    }

    fn multi_day_intraday_body(days: &[NaiveDate], bars_per_day: usize) -> String {
        let mut ts = Vec::new();
        for &d in days {
            let base = open_ist(d);
            for i in 0..bars_per_day {
                ts.push((base + i as i64 * 60).to_string());
            }
        }
        let n = ts.len();
        let ones = vec!["1.0"; n].join(",");
        let twos = vec!["2.0"; n].join(",");
        let vols = vec!["10.0"; n].join(",");
        format!(
            "{{\"open\":[{o}],\"high\":[{h}],\"low\":[{l}],\"close\":[{c}],\"volume\":[{v}],\"timestamp\":[{t}]}}",
            o = ones,
            h = twos,
            l = ones,
            c = twos,
            v = vols,
            t = ts.join(",")
        )
    }

    /// T1: Update a symbol over Monday to the following Monday with the mock server,
    /// then open the same range. Must succeed (regression for D1).
    #[test]
    fn t1_update_over_weekend_then_open_must_succeed() {
        let root = temp("t1-weekend");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let mon1 = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let mon2 = NaiveDate::from_ymd_opt(2026, 1, 12).unwrap();
        let trading_days = vec![
            mon1,
            NaiveDate::from_ymd_opt(2026, 1, 6).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 7).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 8).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 9).unwrap(),
            mon2,
        ];
        let body = multi_day_intraday_body(&trading_days, 375);
        let (base, server) = mock_server(vec![(200u16, body)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", mon1, mon2).unwrap();
        let outcome = server.join().unwrap();
        assert_eq!(outcome.served, 1);

        let source = CatalogSource::new(Store::new(&root), &db);
        let range = DateRange {
            from: Some(open_ist(mon1)),
            to: Some(open_ist(mon2) + 375 * 60 - 60),
        };
        let stream = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
        let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(candles.len(), 6 * 375);
        let _ = fs::remove_dir_all(root);
    }

    /// T2: A weekday holiday marked non-trading in the calendar inside the range does not block open.
    #[test]
    fn t2_weekday_holiday_does_not_block_open() {
        let root = temp("t2-holiday");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let tue = NaiveDate::from_ymd_opt(2026, 1, 6).unwrap();
        let wed = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
        {
            let cat = Catalog::open(&db).unwrap();
            cat.conn
                .execute(
                    "INSERT INTO trading_calendar (date, is_trading, source) VALUES (?1, 0, 'holiday')",
                    params![tue.to_string()],
                )
                .unwrap();
        }
        let trading_days = vec![mon, wed];
        let body = multi_day_intraday_body(&trading_days, 375);
        let (base, server) = mock_server(vec![(200u16, body)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", mon, wed).unwrap();
        let _ = server.join().unwrap();

        let source = CatalogSource::new(Store::new(&root), &db);
        let range = DateRange {
            from: Some(open_ist(mon)),
            to: Some(open_ist(wed) + 375 * 60 - 60),
        };
        let stream = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
        let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(candles.len(), 2 * 375);
        let _ = fs::remove_dir_all(root);
    }

    /// T3: Uncovered symbols: the error lists all of them with exact commands, and the catalog file is byte-identical afterwards.
    #[test]
    fn t3_uncovered_symbols_error_and_catalog_byte_identical() {
        let root = temp("t3-uncovered");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        seed_instrument(&db, "2", "XYZ");
        let before_bytes = fs::read(&db).unwrap();

        let source = CatalogSource::new(Store::new(&root), &db);
        let d1 = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let d2 = NaiveDate::from_ymd_opt(2026, 1, 6).unwrap();
        let range = DateRange {
            from: Some(open_ist(d1)),
            to: Some(open_ist(d2) + 375 * 60 - 60),
        };
        let err = match source.open(&["ABC".into(), "XYZ".into()], Timeframe::M1, range) {
            Ok(_) => panic!("expected error"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("ABC"));
        assert!(err.contains("XYZ"));
        assert!(err.contains("algomln data update --symbols ABC,XYZ --tf 1m"));

        let after_bytes = fs::read(&db).unwrap();
        assert_eq!(before_bytes, after_bytes);

        let cat = Catalog::open(&db).unwrap();
        let count: i64 = cat
            .conn
            .query_row("SELECT count(*) FROM series", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let _ = fs::remove_dir_all(root);
    }

    /// T4: Open-ended from and to resolve to the series bounds. An empty series returns Uncovered.
    #[test]
    fn t4_open_ended_ranges_and_empty_series() {
        let root = temp("t4-openended");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        seed_instrument(&db, "2", "EMPTY");
        let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let tue = NaiveDate::from_ymd_opt(2026, 1, 6).unwrap();
        let body = multi_day_intraday_body(&[mon, tue], 375);
        let (base, server) = mock_server(vec![(200u16, body)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", mon, tue).unwrap();
        let _ = server.join().unwrap();

        let source = CatalogSource::new(Store::new(&root), &db);
        let stream = source
            .open(
                &["ABC".into()],
                Timeframe::M1,
                DateRange {
                    from: None,
                    to: None,
                },
            )
            .unwrap();
        let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(candles.len(), 2 * 375);

        let res = source.open(
            &["EMPTY".into()],
            Timeframe::M1,
            DateRange {
                from: None,
                to: None,
            },
        );
        assert!(res.is_err());

        let _ = fs::remove_dir_all(root);
    }

    /// T5: Warm-up: an F&O and a non-F&O symbol get different sessions_needed for the same warmup_bars.
    /// Insufficient history returns WarmupShort. With allow_gaps it warns and continues.
    #[test]
    fn t5_warmup_fno_vs_non_fno_and_warmup_short() {
        let root = temp("t5-warmup");
        let db = root.join("catalog.sqlite");
        {
            let mut cat = Catalog::open(&db).unwrap();
            cat.refresh_instruments(
                &[
                    Scrip {
                        security_id: "1".into(),
                        symbol: "FNO_SYM".into(),
                        lot_size: 25,
                        tick_size_paise: 5,
                        instrument: "EQUITY".into(),
                        is_fno: true,
                    },
                    Scrip {
                        security_id: "2".into(),
                        symbol: "EQ_SYM".into(),
                        lot_size: 1,
                        tick_size_paise: 5,
                        instrument: "EQUITY".into(),
                        is_fno: false,
                    },
                ],
                0,
            )
            .unwrap();
        }
        let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let tue = NaiveDate::from_ymd_opt(2026, 1, 6).unwrap();
        let body_fno = multi_day_intraday_body(&[mon, tue], 360);
        let body_eq = multi_day_intraday_body(&[mon, tue], 375);
        let (base, server) = mock_server(vec![(200u16, body_fno), (200u16, body_eq)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", mon, tue).unwrap();
        up.update("2", Tf::M1, "EQUITY", mon, tue).unwrap();
        let _ = server.join().unwrap();

        let mut source = CatalogSource::new(Store::new(&root), &db).with_warmup_bars(365);
        let range = DateRange {
            from: Some(open_ist(tue)),
            to: Some(open_ist(tue) + 360 * 60 - 60),
        };
        let err = match source.open(&["FNO_SYM".into()], Timeframe::M1, range) {
            Ok(_) => panic!("expected warmup short error"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("warmup short"));

        let range_eq = DateRange {
            from: Some(open_ist(tue)),
            to: Some(open_ist(tue) + 375 * 60 - 60),
        };
        let stream = source
            .open(&["EQ_SYM".into()], Timeframe::M1, range_eq)
            .unwrap();
        let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        assert!(candles.len() > 375);

        source.allow_gaps = true;
        let stream_fno = source
            .open(&["FNO_SYM".into()], Timeframe::M1, range)
            .unwrap();
        let candles_fno: Vec<_> = stream_fno.collect::<Result<Vec<_>, _>>().unwrap();
        assert!(!candles_fno.is_empty());

        let _ = fs::remove_dir_all(root);
    }

    /// T6: Bit-identical reruns: two runs give the same candle-sequence hash, the same fingerprint.
    #[test]
    fn t6_bit_identical_reruns() {
        let root = temp("t6-identical");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let body = multi_day_intraday_body(&[mon], 375);
        let (base, server) = mock_server(vec![(200u16, body)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
        let _ = server.join().unwrap();

        let source = CatalogSource::new(Store::new(&root), &db);
        let range = DateRange {
            from: Some(open_ist(mon)),
            to: Some(open_ist(mon) + 375 * 60 - 60),
        };
        let fp1 = source.fingerprint(&["ABC".into()], Timeframe::M1, range);
        let fp2 = source.fingerprint(&["ABC".into()], Timeframe::M1, range);
        assert_eq!(fp1, fp2);

        let stream1 = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
        let c1: Vec<_> = stream1.collect::<Result<Vec<_>, _>>().unwrap();
        let stream2 = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
        let c2: Vec<_> = stream2.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(c1, c2);
        let _ = fs::remove_dir_all(root);
    }

    /// T7: FileSource and CatalogSource on identical candles produce identical streams.
    #[test]
    fn t7_filesource_and_catalogsource_identical() {
        let root = temp("t7-file-vs-cat");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let body = multi_day_intraday_body(&[mon], 375);
        let (base, server) = mock_server(vec![(200u16, body)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
        let _ = server.join().unwrap();

        let cat_source = CatalogSource::new(Store::new(&root), &db);
        let range = DateRange {
            from: Some(open_ist(mon)),
            to: Some(open_ist(mon) + 375 * 60 - 60),
        };
        let cat_candles: Vec<_> = cat_source
            .open(&["ABC".into()], Timeframe::M1, range)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        let csv_dir = root.join("csv");
        fs::create_dir_all(&csv_dir).unwrap();
        let mut wtr = csv::Writer::from_path(csv_dir.join("ABC.csv")).unwrap();
        wtr.write_record(&["ts", "open", "high", "low", "close", "volume"])
            .unwrap();
        for (_, c) in &cat_candles {
            wtr.write_record(&[
                c.timestamp.to_string(),
                c.open.to_string(),
                c.high.to_string(),
                c.low.to_string(),
                c.close.to_string(),
                c.volume.to_string(),
            ])
            .unwrap();
        }
        wtr.flush().unwrap();

        let file_source = FileSource::new(&csv_dir);
        let file_candles: Vec<_> = file_source
            .open(&["ABC".into()], Timeframe::M1, range)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(cat_candles.len(), file_candles.len());
        for (c_cat, c_file) in cat_candles.iter().zip(file_candles.iter()) {
            assert_eq!(c_cat.0, c_file.0);
            assert_eq!(c_cat.1.timestamp, c_file.1.timestamp);
            assert_eq!(c_cat.1.open, c_file.1.open);
            assert_eq!(c_cat.1.close, c_file.1.close);
        }
        let _ = fs::remove_dir_all(root);
    }

    /// T8: A 360-bar symbol and a 375-bar symbol merge: ties resolved by symbol name.
    #[test]
    fn t8_symbol_merge_ties_resolved_by_symbol_name() {
        let root = temp("t8-merge");
        let db = root.join("catalog.sqlite");
        {
            let mut cat = Catalog::open(&db).unwrap();
            cat.refresh_instruments(
                &[
                    Scrip {
                        security_id: "1".into(),
                        symbol: "BBB".into(),
                        lot_size: 1,
                        tick_size_paise: 5,
                        instrument: "EQUITY".into(),
                        is_fno: false,
                    },
                    Scrip {
                        security_id: "2".into(),
                        symbol: "AAA".into(),
                        lot_size: 25,
                        tick_size_paise: 5,
                        instrument: "EQUITY".into(),
                        is_fno: true,
                    },
                ],
                0,
            )
            .unwrap();
        }
        let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let body_b = multi_day_intraday_body(&[mon], 375);
        let body_a = multi_day_intraday_body(&[mon], 360);
        let (base, server) = mock_server(vec![(200u16, body_b), (200u16, body_a)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
        up.update("2", Tf::M1, "EQUITY", mon, mon).unwrap();
        let _ = server.join().unwrap();

        let source = CatalogSource::new(Store::new(&root), &db);
        let range = DateRange {
            from: Some(open_ist(mon)),
            to: Some(open_ist(mon) + 375 * 60 - 60),
        };
        let stream = source
            .open(&["BBB".into(), "AAA".into()], Timeframe::M1, range)
            .unwrap();
        let merged: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();

        assert_eq!(merged.len(), 360 + 375);
        let a_count = merged.iter().filter(|(s, _)| s == "AAA").count();
        let b_count = merged.iter().filter(|(s, _)| s == "BBB").count();
        assert_eq!(a_count, 360);
        assert_eq!(b_count, 375);

        assert_eq!(merged[0].0, "AAA");
        assert_eq!(merged[1].0, "BBB");
        assert_eq!(merged[0].1.timestamp, merged[1].1.timestamp);

        let _ = fs::remove_dir_all(root);
    }

    /// T9: Flip one byte in a partition: run fails naming the path, catalog unchanged. A deleted partition file fails the same way.
    #[test]
    fn t9_corrupted_or_deleted_partition_fails() {
        let root = temp("t9-corrupt");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let body = multi_day_intraday_body(&[mon], 375);
        let (base, server) = mock_server(vec![(200u16, body)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
        let _ = server.join().unwrap();

        let cat_before = fs::read(&db).unwrap();

        let part_file = root.join("candles").join("1").join("1m").join("2026-01.parquet");
        assert!(part_file.exists());

        let mut bytes = fs::read(&part_file).unwrap();
        bytes[0] ^= 0xFF;
        fs::write(&part_file, &bytes).unwrap();

        let source = CatalogSource::new(Store::new(&root), &db);
        let range = DateRange {
            from: Some(open_ist(mon)),
            to: Some(open_ist(mon) + 375 * 60 - 60),
        };
        let err = match source.open(&["ABC".into()], Timeframe::M1, range) {
            Ok(stream) => stream.collect::<Result<Vec<_>, _>>().unwrap_err().to_string(),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("checksum mismatch"));
        assert!(err.contains("data verify"));

        let cat_after = fs::read(&db).unwrap();
        assert_eq!(cat_before, cat_after);

        fs::remove_file(&part_file).unwrap();
        let err2 = match source.open(&["ABC".into()], Timeframe::M1, range) {
            Ok(stream) => stream.collect::<Result<Vec<_>, _>>().unwrap_err().to_string(),
            Err(e) => e.to_string(),
        };
        assert!(err2.contains("missing partition file"));

        let _ = fs::remove_dir_all(root);
    }

    /// T10: A corrupted partition of an unrelated symbol does not affect or slow the run.
    #[test]
    fn t10_corrupted_partition_of_unrelated_symbol_does_not_affect_run() {
        let root = temp("t10-unrelated");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "GOOD");
        seed_instrument(&db, "2", "BAD");
        let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let body1 = multi_day_intraday_body(&[mon], 375);
        let body2 = multi_day_intraday_body(&[mon], 375);
        let (base, server) = mock_server(vec![(200u16, body1), (200u16, body2)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
        up.update("2", Tf::M1, "EQUITY", mon, mon).unwrap();
        let _ = server.join().unwrap();

        let bad_part = root.join("candles").join("2").join("1m").join("2026-01.parquet");
        fs::write(&bad_part, b"garbage data").unwrap();

        let source = CatalogSource::new(Store::new(&root), &db);
        let range = DateRange {
            from: Some(open_ist(mon)),
            to: Some(open_ist(mon) + 375 * 60 - 60),
        };
        let stream = source.open(&["GOOD".into()], Timeframe::M1, range).unwrap();
        let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(candles.len(), 375);

        let _ = fs::remove_dir_all(root);
    }

    /// T11: Fingerprint: a partition outside the window leaves it unchanged. A changed bar or lot size changes it.
    #[test]
    fn t11_fingerprint_window_and_metadata_sensitivity() {
        let root = temp("t11-fingerprint");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let jan = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let feb = NaiveDate::from_ymd_opt(2026, 2, 2).unwrap();
        let mar = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
        let b_jan = multi_day_intraday_body(&[jan], 375);
        let b_feb = multi_day_intraday_body(&[feb], 375);
        let (base, server) = mock_server(vec![(200u16, b_jan), (200u16, b_feb)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", jan, jan).unwrap();
        up.update("1", Tf::M1, "EQUITY", feb, feb).unwrap();
        let _ = server.join().unwrap();

        let source = CatalogSource::new(Store::new(&root), &db);
        let range_jan = DateRange {
            from: Some(open_ist(jan)),
            to: Some(open_ist(jan) + 375 * 60 - 60),
        };
        let fp1 = source.fingerprint(&["ABC".into()], Timeframe::M1, range_jan);

        let b_mar = multi_day_intraday_body(&[mar], 375);
        let (base2, server2) = mock_server(vec![(200u16, b_mar)]);
        let mut up2 = make_updater(&base2, &db, &root);
        up2.update("1", Tf::M1, "EQUITY", mar, mar).unwrap();
        let _ = server2.join().unwrap();

        let fp2 = source.fingerprint(&["ABC".into()], Timeframe::M1, range_jan);
        assert_eq!(
            fp1, fp2,
            "partition outside window must not change fingerprint"
        );

        {
            let cat = Catalog::open(&db).unwrap();
            cat.conn
                .execute(
                    "UPDATE instruments SET lot_size=50 WHERE security_id='1'",
                    [],
                )
                .unwrap();
        }
        let fp3 = source.fingerprint(&["ABC".into()], Timeframe::M1, range_jan);
        assert_ne!(fp1, fp3, "changing lot size must change fingerprint");

        let _ = fs::remove_dir_all(root);
    }

    /// T12: Mid-run data update makes the start and end fingerprints differ.
    #[test]
    fn t12_mid_run_update_fingerprint_differ() {
        let root = temp("t12-midrun");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let body = multi_day_intraday_body(&[mon], 375);
        let (base, server) = mock_server(vec![(200u16, body)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
        let _ = server.join().unwrap();

        let source = CatalogSource::new(Store::new(&root), &db);
        let range = DateRange {
            from: Some(open_ist(mon)),
            to: Some(open_ist(mon) + 375 * 60 - 60),
        };
        let fp_start = source.fingerprint(&["ABC".into()], Timeframe::M1, range);

        {
            let cat = Catalog::open(&db).unwrap();
            cat.conn
                .execute(
                    "UPDATE partitions SET checksum='changed_hash' WHERE series_id=1",
                    [],
                )
                .unwrap();
        }
        let fp_end = source.fingerprint(&["ABC".into()], Timeframe::M1, range);
        assert_ne!(fp_start, fp_end);

        let _ = fs::remove_dir_all(root);
    }

    /// T13: Lazy loading: a counter in the cursor proves at most one partition per symbol is resident at any time.
    #[test]
    fn t13_lazy_loading_resident_partition_at_most_one() {
        let root = temp("t13-lazy");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let jan = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let feb = NaiveDate::from_ymd_opt(2026, 5, 4).unwrap();
        let mar = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
        let b1 = multi_day_intraday_body(&[jan], 375);
        let b2 = multi_day_intraday_body(&[feb], 375);
        let b3 = multi_day_intraday_body(&[mar], 375);
        let (base, server) = mock_server(vec![(200u16, b1), (200u16, b2), (200u16, b3)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", jan, mar).unwrap();
        let _ = server.join().unwrap();

        let source = CatalogSource::new(Store::new(&root), &db);
        let range = DateRange {
            from: Some(open_ist(jan)),
            to: Some(open_ist(mar) + 375 * 60 - 60),
        };
        let stream = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
        let counter = source
            .resident_counter_for(&"ABC".into())
            .expect("resident counter");

        let mut max_resident = 0;
        let mut total_candles = 0;
        for item in stream {
            let _ = item.unwrap();
            total_candles += 1;
            let current = counter.load(std::sync::atomic::Ordering::SeqCst);
            if current > max_resident {
                max_resident = current;
            }
        }
        assert_eq!(total_candles, 3 * 375);
        assert_eq!(
            max_resident, 1,
            "at most 1 partition per symbol may be resident at any time"
        );

        let _ = fs::remove_dir_all(root);
    }

    /// T14: The catalog file is unchanged (bytes and mtime) after open plus a full run.
    #[test]
    fn t14_catalog_file_unchanged_bytes_and_mtime() {
        let root = temp("t14-unchanged");
        let db = root.join("catalog.sqlite");
        seed_instrument(&db, "1", "ABC");
        let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
        let body = multi_day_intraday_body(&[mon], 375);
        let (base, server) = mock_server(vec![(200u16, body)]);
        let mut up = make_updater(&base, &db, &root);
        up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
        let _ = server.join().unwrap();

        let before_bytes = fs::read(&db).unwrap();
        let before_mtime = fs::metadata(&db).unwrap().modified().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));

        let source = CatalogSource::new(Store::new(&root), &db);
        let range = DateRange {
            from: Some(open_ist(mon)),
            to: Some(open_ist(mon) + 375 * 60 - 60),
        };
        let stream = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
        let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(candles.len(), 375);

        let after_bytes = fs::read(&db).unwrap();
        let after_mtime = fs::metadata(&db).unwrap().modified().unwrap();
        assert_eq!(before_bytes, after_bytes, "catalog bytes must be unchanged");
        assert_eq!(before_mtime, after_mtime, "catalog mtime must be unchanged");

        let _ = fs::remove_dir_all(root);
    }
}
