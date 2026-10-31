//! Deterministic, local historical simulation primitives.
//!
//! This module depends on the strategy engine and execution port only. Data
//! sources are replaceable; the file source never performs network access.
use crate::{
    engine::{Engine, LevelFrame, Program, Symbol},
    indicators::Candle,
    trader::{
        ExecutionPort, Mode, OrderEvent, OrderId, OrderReason, OrderRequest, PortError, Side,
        Trader, TraderConfig, TraderEvent,
    },
};
use arrow_array::{Array, Float64Array, Int64Array, RecordBatch};
use chrono::{DateTime, Datelike, FixedOffset, TimeZone};
use parquet::arrow::arrow_reader::{ParquetRecordBatchReader, ParquetRecordBatchReaderBuilder};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap, HashMap},
    fs::File,
    io::Read,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Timeframe {
    M1,
    M5,
    M15,
    M25,
    M60,
    D1,
}
impl Timeframe {
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
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateRange {
    pub from: Option<i64>,
    pub to: Option<i64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint(pub String);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GapPolicy {
    None,
    FillFlatInSession,
}
#[derive(Debug, Error)]
pub enum SourceError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{file}, row {row}: {message}")]
    Invalid {
        file: PathBuf,
        row: usize,
        message: String,
    },
    #[error("{0}")]
    Other(String),
}
pub type CandleStream = Box<dyn Iterator<Item = Result<(Symbol, Candle), SourceError>> + Send>;

/// Reproducible candles for exercising strategy behavior without external data.
/// The generated series includes an upward trend, flat sessions, one missing
/// symbol interval, intraday gaps, and a deterministic volatile session.
pub fn seeded_synthetic_candles(
    seed: u64,
    symbols: &[Symbol],
    sessions: usize,
) -> Vec<(Symbol, Candle)> {
    let mut state = seed;
    let mut rows = Vec::new();
    const SESSION_OPEN: i64 = 9 * 3600 + 15 * 60;
    for day in 0..sessions {
        let date = chrono::NaiveDate::from_ymd_opt(2024, 1, 2)
            .unwrap()
            .checked_add_days(chrono::Days::new(day as u64))
            .unwrap();
        // Skip weekends to create realistic session gaps in the input stream.
        if matches!(date.weekday(), chrono::Weekday::Sat | chrono::Weekday::Sun) {
            continue;
        }
        let day_start =
            chrono::NaiveDateTime::new(date, chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap())
                .and_utc()
                .timestamp();
        for (symbol_index, symbol) in symbols.iter().enumerate() {
            let mut price = 100.0 + symbol_index as f64 * 20.0;
            for bar in 0..12 {
                // One missing symbol interval, repeated deterministically.
                if symbol_index == 1 && day == 1 && bar == 5 {
                    continue;
                }
                // Flat opening/closing sections and a volatile middle session.
                let change = if !(2..=9).contains(&bar) {
                    0.0
                } else if day == 2 && (5..=7).contains(&bar) {
                    if bar % 2 == 0 {
                        8.0
                    } else {
                        -7.0
                    }
                } else {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    (state % 401) as f64 / 100.0 - 2.0 + (bar as f64 * 0.1)
                };
                let open = price;
                price = (price + change).max(1.0);
                let ts = day_start + SESSION_OPEN + bar * 60;
                rows.push((
                    symbol.clone(),
                    Candle {
                        timestamp: ts,
                        open,
                        high: open.max(price)
                            + if day == 2 && (5..=7).contains(&bar) {
                                3.0
                            } else {
                                0.1
                            },
                        low: open.min(price)
                            - if day == 2 && (5..=7).contains(&bar) {
                                3.0
                            } else {
                                0.1
                            },
                        close: price,
                        volume: if !(2..=9).contains(&bar) {
                            0.0
                        } else {
                            (state % 1000 + 1) as f64
                        },
                    },
                ));
            }
        }
    }
    rows.sort_by(|a, b| (a.1.timestamp, &a.0).cmp(&(b.1.timestamp, &b.0)));
    rows
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceStats {
    pub skipped_bad: BTreeMap<Symbol, u64>,
    pub missing_candles: BTreeMap<Symbol, u64>,
}
pub trait CandleSource: Send + Sync {
    fn open(
        &self,
        symbols: &[Symbol],
        tf: Timeframe,
        range: DateRange,
    ) -> Result<CandleStream, SourceError>;
    fn fingerprint(&self, symbols: &[Symbol], tf: Timeframe, range: DateRange) -> Fingerprint;
    fn stats(&self) -> SourceStats {
        SourceStats::default()
    }
    fn instrument_rules(
        &self,
        _symbols: &[Symbol],
    ) -> BTreeMap<Symbol, crate::trader::InstrumentRule> {
        BTreeMap::new()
    }
    fn tick_sizes_paise(&self, _symbols: &[Symbol]) -> BTreeMap<Symbol, i64> {
        BTreeMap::new()
    }
}
fn limit_to_warmup_window(mut input: CandleStream, from: Option<i64>, bars: usize) -> CandleStream {
    let Some(from) = from else { return input };
    let mut tails = BTreeMap::<Symbol, std::collections::VecDeque<Candle>>::new();
    let mut first = None;
    let mut early_error = None;
    for item in input.by_ref() {
        match item {
            Ok((symbol, candle)) if candle.timestamp < from => {
                if bars > 0 {
                    let q = tails.entry(symbol).or_default();
                    q.push_back(candle);
                    if q.len() > bars {
                        q.pop_front();
                    }
                }
            }
            Ok(x) => {
                first = Some(x);
                break;
            }
            Err(e) => {
                early_error = Some(e);
                break;
            }
        }
    }
    if let Some(error) = early_error {
        return Box::new(std::iter::once(Err(error)));
    }
    let Some(first) = first else {
        return Box::new(std::iter::empty());
    };
    let mut warm = Vec::new();
    for (symbol, tail) in tails {
        warm.extend(tail.into_iter().map(|c| (symbol.clone(), c)));
    }
    warm.sort_by(|a, b| (a.1.timestamp, &a.0).cmp(&(b.1.timestamp, &b.0)));
    Box::new(
        warm.into_iter()
            .map(Ok)
            .chain(std::iter::once(Ok(first)))
            .chain(input),
    )
}

/// One-row-per-symbol merge. The heap contains at most one decoded candle per
/// input, keeping the file-backed stream bounded by the number of symbols.
pub struct FileSource {
    pub dir: PathBuf,
    pub csv_tz: Option<String>,
    pub skip_bad: bool,
    pub gap_policy: GapPolicy,
    stats: Arc<Mutex<SourceStats>>,
}
impl FileSource {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            csv_tz: None,
            skip_bad: false,
            gap_policy: GapPolicy::None,
            stats: Arc::new(Mutex::new(SourceStats::default())),
        }
    }
}
struct CsvCursor {
    symbol: Symbol,
    path: PathBuf,
    reader: csv::Reader<File>,
    row: usize,
    previous: Option<i64>,
    tz: Option<CsvTimezone>,
    skip_bad: bool,
    stats: Arc<Mutex<SourceStats>>,
}
struct ParquetCursor {
    symbol: Symbol,
    path: PathBuf,
    reader: ParquetRecordBatchReader,
    batch: Option<RecordBatch>,
    row: usize,
    row_in_batch: usize,
    columns: [usize; 6],
    previous: Option<i64>,
    skip_bad: bool,
    stats: Arc<Mutex<SourceStats>>,
}
enum DataCursor {
    Csv(CsvCursor),
    Parquet(ParquetCursor),
}
struct GapCursor {
    inner: DataCursor,
    symbol: Symbol,
    policy: GapPolicy,
    step: i64,
    pending: Option<Candle>,
    previous: Option<Candle>,
    gaps: std::collections::VecDeque<Candle>,
    stats: Arc<Mutex<SourceStats>>,
}
impl GapCursor {
    fn next(&mut self) -> Option<Result<(Symbol, Candle), SourceError>> {
        loop {
            if let Some(c) = self.gaps.pop_front() {
                self.previous = Some(c);
                return Some(Ok((self.symbol.clone(), c)));
            }
            let (current, may_fill) = if let Some(c) = self.pending.take() {
                (Some(Ok((self.symbol.clone(), c))), false)
            } else {
                (self.inner.next(), true)
            };
            let (s, c) = match current? {
                Ok(x) => x,
                Err(e) => return Some(Err(e)),
            };
            if may_fill {
                if let Some(prev) = self.previous {
                    // Gap filling is intraday-only and confined to a single trading
                    // session. Daily (and coarser) series are never filled: a gap
                    // between two daily bars spans non-trading days (weekends,
                    // holidays), and the sim carries no trading calendar to tell a
                    // holiday apart from genuinely missing data, so a flat fill there
                    // would fabricate bars and corrupt the backtest. Overnight /
                    // cross-session gaps are skipped for the same reason (markets are
                    // closed, so no bars are "missing") — hence prev and c must share
                    // an IST day and only in-session timestamps are counted/filled.
                    let intraday = self.step < 86_400;
                    let same_session = ist_day(prev.timestamp) == ist_day(c.timestamp);
                    if intraday && same_session {
                        let mut ts = prev.timestamp.saturating_add(self.step);
                        let mut count = 0;
                        while ts < c.timestamp && in_ist_session(ts) {
                            count += 1;
                            if self.policy == GapPolicy::FillFlatInSession {
                                self.gaps.push_back(Candle {
                                    timestamp: ts,
                                    open: prev.close,
                                    high: prev.close,
                                    low: prev.close,
                                    close: prev.close,
                                    volume: 0.0,
                                });
                            }
                            ts = ts.saturating_add(self.step);
                        }
                        if count > 0 {
                            if let Ok(mut stats) = self.stats.lock() {
                                *stats
                                    .missing_candles
                                    .entry(self.symbol.clone())
                                    .or_default() += count;
                            }
                        }
                        if !self.gaps.is_empty() {
                            self.pending = Some(c);
                            continue;
                        }
                    }
                }
            }
            self.previous = Some(c);
            return Some(Ok((s, c)));
        }
    }
}
fn in_ist_session(ts: i64) -> bool {
    let local = (ts + 19800).rem_euclid(86400);
    (9 * 3600 + 15 * 60..15 * 3600 + 30 * 60).contains(&local)
}
#[derive(Clone, Copy)]
enum CsvTimezone {
    Named(chrono_tz::Tz),
    Offset(FixedOffset),
}
fn timeframe_seconds(tf: Timeframe) -> i64 {
    match tf {
        Timeframe::M1 => 60,
        Timeframe::M5 => 300,
        Timeframe::M15 => 900,
        Timeframe::M25 => 1500,
        Timeframe::M60 => 3600,
        Timeframe::D1 => 86400,
    }
}
impl CsvCursor {
    fn next(&mut self) -> Option<Result<(Symbol, Candle), SourceError>> {
        loop {
            let mut record = csv::StringRecord::new();
            match self.reader.read_record(&mut record) {
                Ok(false) => return None,
                Err(e) if self.skip_bad => {
                    self.row += 1;
                    if let Ok(mut stats) = self.stats.lock() {
                        *stats.skipped_bad.entry(self.symbol.clone()).or_default() += 1;
                    }
                    let _ = e;
                    continue;
                }
                Err(e) => {
                    return Some(Err(SourceError::Other(format!(
                        "{} row {}: {e}",
                        self.path.display(),
                        self.row + 1
                    ))))
                }
                Ok(true) => {}
            }
            self.row += 1;
            if self.row == 1 && record.get(0).is_some_and(|value| value.trim() == "ts") {
                continue;
            }
            let invalid = |message: String| SourceError::Invalid {
                file: self.path.clone(),
                row: self.row,
                message,
            };
            let parsed = (|| -> Result<Candle, String> {
                if record.len() != 6 {
                    return Err("expected ts,open,high,low,close,volume".into());
                }
                let ts =
                    parse_timestamp(record.get(0).unwrap(), self.tz).map_err(|e| e.to_string())?;
                let mut v = [0.0; 5];
                for (i, x) in record.iter().skip(1).enumerate() {
                    v[i] = x.parse().map_err(|_| format!("invalid number {x:?}"))?;
                }
                let c = Candle {
                    timestamp: ts,
                    open: v[0],
                    high: v[1],
                    low: v[2],
                    close: v[3],
                    volume: v[4],
                };
                c.is_valid(None).map_err(str::to_owned)?;
                if [c.open, c.high, c.low, c.close].iter().any(|p| *p <= 0.0)
                    || c.low > c.open.min(c.close)
                    || c.high < c.open.max(c.close)
                {
                    return Err(
                        "prices must be positive and low/high must bound open and close".into(),
                    );
                }
                if self.previous.is_some_and(|p| ts <= p) {
                    return Err("timestamp must be strictly increasing".into());
                }
                self.previous = Some(ts);
                Ok(c)
            })();
            match parsed {
                Ok(c) => return Some(Ok((self.symbol.clone(), c))),
                Err(_msg) if self.skip_bad => {
                    if let Ok(mut stats) = self.stats.lock() {
                        *stats.skipped_bad.entry(self.symbol.clone()).or_default() += 1;
                    }
                    continue;
                }
                Err(msg) => return Some(Err(invalid(msg))),
            }
        }
    }
}
impl ParquetCursor {
    fn next(&mut self) -> Option<Result<(Symbol, Candle), SourceError>> {
        loop {
            if self
                .batch
                .as_ref()
                .map_or(true, |b| self.row_in_batch >= b.num_rows())
            {
                match self.reader.next() {
                    Some(Ok(batch)) => {
                        self.batch = Some(batch);
                        self.row_in_batch = 0;
                    }
                    Some(Err(e)) if self.skip_bad => {
                        self.row += 1;
                        if let Ok(mut stats) = self.stats.lock() {
                            *stats.skipped_bad.entry(self.symbol.clone()).or_default() += 1;
                        }
                        let _ = e;
                        continue;
                    }
                    Some(Err(e)) => {
                        return Some(Err(SourceError::Other(format!(
                            "{} row {}: {e}",
                            self.path.display(),
                            self.row + 1
                        ))))
                    }
                    None => return None,
                }
            }
            let batch = self.batch.as_ref().unwrap();
            let i = self.row_in_batch;
            self.row_in_batch += 1;
            self.row += 1;
            let invalid = |message: String| SourceError::Invalid {
                file: self.path.clone(),
                row: self.row,
                message,
            };
            let parsed = (|| -> Result<Candle, String> {
                let ts = batch
                    .column(self.columns[0])
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .ok_or("ts column must be i64")?;
                let mut vals = [0.0; 5];
                for j in 1..6 {
                    let array = batch
                        .column(self.columns[j])
                        .as_any()
                        .downcast_ref::<Float64Array>()
                        .ok_or_else(|| {
                            format!(
                                "{} column must be f64",
                                ["open", "high", "low", "close", "volume"][j - 1]
                            )
                        })?;
                    if array.is_null(i) {
                        return Err("null values are not allowed".into());
                    }
                    vals[j - 1] = array.value(i);
                }
                if ts.is_null(i) {
                    return Err("null timestamp is not allowed".into());
                }
                let c = Candle {
                    timestamp: ts.value(i),
                    open: vals[0],
                    high: vals[1],
                    low: vals[2],
                    close: vals[3],
                    volume: vals[4],
                };
                c.is_valid(None).map_err(str::to_owned)?;
                if [c.open, c.high, c.low, c.close].iter().any(|p| *p <= 0.0)
                    || c.low > c.open.min(c.close)
                    || c.high < c.open.max(c.close)
                {
                    return Err(
                        "prices must be positive and low/high must bound open and close".into(),
                    );
                }
                if self.previous.is_some_and(|p| c.timestamp <= p) {
                    return Err("timestamp must be strictly increasing".into());
                }
                self.previous = Some(c.timestamp);
                Ok(c)
            })();
            match parsed {
                Ok(c) => return Some(Ok((self.symbol.clone(), c))),
                Err(_e) if self.skip_bad => {
                    if let Ok(mut stats) = self.stats.lock() {
                        *stats.skipped_bad.entry(self.symbol.clone()).or_default() += 1;
                    }
                    continue;
                }
                Err(e) => return Some(Err(invalid(e))),
            }
        }
    }
}
impl DataCursor {
    fn next(&mut self) -> Option<Result<(Symbol, Candle), SourceError>> {
        match self {
            Self::Csv(c) => c.next(),
            Self::Parquet(c) => c.next(),
        }
    }
}
fn parse_timestamp(s: &str, tz: Option<CsvTimezone>) -> Result<i64, String> {
    if let Ok(n) = s.parse::<i64>() {
        return Ok(n);
    }
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.timestamp());
    }
    let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|_| "timestamp must be epoch seconds or RFC3339 with offset".to_string())?;
    let local = match tz.ok_or_else(|| "naive timestamp requires --csv-tz".to_string())? {
        CsvTimezone::Named(tz) => tz
            .from_local_datetime(&naive)
            .single()
            .map(|d| d.timestamp()),
        CsvTimezone::Offset(tz) => tz
            .from_local_datetime(&naive)
            .single()
            .map(|d| d.timestamp()),
    };
    local.ok_or_else(|| "ambiguous or invalid local timestamp".into())
}
impl CandleSource for FileSource {
    fn open(
        &self,
        symbols: &[Symbol],
        tf: Timeframe,
        range: DateRange,
    ) -> Result<CandleStream, SourceError> {
        if let Ok(mut stats) = self.stats.lock() {
            *stats = SourceStats::default();
        }
        let tz = match self.csv_tz.as_deref() {
            None => None,
            Some(x) => Some(if let Some(offset) = parse_fixed_tz(x) {
                CsvTimezone::Offset(offset)
            } else {
                CsvTimezone::Named(
                    x.parse::<chrono_tz::Tz>()
                        .map_err(|_| SourceError::Other(format!("unsupported timezone {x:?}")))?,
                )
            }),
        };
        let mut cursors: Vec<GapCursor> = Vec::new();
        for s in symbols {
            let csv = self.dir.join(format!("{s}.csv"));
            let pq = self.dir.join(format!("{s}.parquet"));
            let path = if csv.exists() {
                csv
            } else if pq.exists() {
                pq
            } else if symbols.len() == 1 && self.dir.is_file() {
                self.dir.clone()
            } else {
                return Err(SourceError::Other(format!("missing data file for {s}")));
            };
            let inner = if path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.eq_ignore_ascii_case("parquet"))
            {
                let builder = ParquetRecordBatchReaderBuilder::try_new(File::open(&path)?)
                    .map_err(|e| SourceError::Other(format!("{}: {e}", path.display())))?;
                let schema = builder.schema();
                let names = ["ts", "open", "high", "low", "close", "volume"];
                let mut columns = [0usize; 6];
                for (i, name) in names.iter().enumerate() {
                    columns[i] = schema.index_of(name).map_err(|_| {
                        SourceError::Other(format!(
                            "{}: missing Parquet column {name}",
                            path.display()
                        ))
                    })?;
                }
                let reader = builder
                    .with_batch_size(1024)
                    .build()
                    .map_err(|e| SourceError::Other(format!("{}: {e}", path.display())))?;
                DataCursor::Parquet(ParquetCursor {
                    symbol: s.clone(),
                    path: path.clone(),
                    reader,
                    batch: None,
                    row: 0,
                    row_in_batch: 0,
                    columns,
                    previous: None,
                    skip_bad: self.skip_bad,
                    stats: self.stats.clone(),
                })
            } else {
                DataCursor::Csv(CsvCursor {
                    symbol: s.clone(),
                    path: path.clone(),
                    reader: csv::ReaderBuilder::new()
                        .has_headers(false)
                        .from_reader(File::open(&path)?),
                    row: 0,
                    previous: None,
                    tz,
                    skip_bad: self.skip_bad,
                    stats: self.stats.clone(),
                })
            };
            cursors.push(GapCursor {
                inner,
                symbol: s.clone(),
                policy: self.gap_policy,
                step: timeframe_seconds(tf),
                pending: None,
                previous: None,
                gaps: Default::default(),
                stats: self.stats.clone(),
            });
        }
        let mut heap: BinaryHeap<Reverse<(i64, Symbol, usize)>> = BinaryHeap::new();
        let mut heads: Vec<Option<Candle>> = vec![None; cursors.len()];
        for (i, c) in cursors.iter_mut().enumerate() {
            if let Some((_, x)) = c.next().transpose()? {
                heads[i] = Some(x);
                heap.push(Reverse((x.timestamp, c.symbol.clone(), i)));
            }
        }
        let iter = std::iter::from_fn(move || loop {
            let Reverse((ts, s, i)) = heap.pop()?;
            let c = heads[i].take().expect("heap head");
            let mut out = None;
            if range.from.map_or(true, |x| ts >= x) && range.to.map_or(true, |x| ts <= x) {
                out = Some(Ok((s, c)));
            }
            match cursors[i].next() {
                Some(Ok((ns, nc))) => {
                    heads[i] = Some(nc);
                    heap.push(Reverse((nc.timestamp, ns, i)))
                }
                Some(Err(e)) => return Some(Err(e)),
                None => {}
            }
            if out.is_some() {
                return out;
            }
        });
        Ok(Box::new(iter))
    }
    fn fingerprint(&self, symbols: &[Symbol], tf: Timeframe, range: DateRange) -> Fingerprint {
        // Stable FNV-1a over request metadata and source file bytes.
        let mut h = 0xcbf29ce484222325u64;
        hash_feed(&mut h, tf.as_str().as_bytes());
        hash_feed(
            &mut h,
            format!(
                "{:?}{:?}{:?}{:?}{:?}",
                range.from, range.to, self.csv_tz, self.skip_bad, self.gap_policy
            )
            .as_bytes(),
        );
        for s in symbols {
            hash_feed(&mut h, s.as_bytes());
            if self.dir.is_file() && symbols.len() == 1 {
                hash_file(&mut h, &self.dir);
            } else {
                for ext in ["csv", "parquet"] {
                    hash_file(&mut h, &self.dir.join(format!("{s}.{ext}")));
                }
            }
        }
        Fingerprint(format!("fnv1a64:{h:016x}"))
    }
    fn stats(&self) -> SourceStats {
        self.stats.lock().map(|x| x.clone()).unwrap_or_default()
    }
}
fn hash_feed(h: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *h ^= *byte as u64;
        *h = h.wrapping_mul(0x100000001b3);
    }
}
fn hash_file(h: &mut u64, path: &std::path::Path) {
    if let Ok(mut file) = File::open(path) {
        let mut buffer = [0u8; 64 * 1024];
        loop {
            match file.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => hash_feed(h, &buffer[..n]),
            }
        }
    }
}
fn parse_fixed_tz(s: &str) -> Option<FixedOffset> {
    let sign = if s.starts_with('-') { -1 } else { 1 };
    let p = s.trim_start_matches(['+', '-']);
    let mut a = p.split(':');
    let h: i32 = a.next()?.parse().ok()?;
    let m: i32 = a.next()?.parse().ok()?;
    if m >= 60 || h >= 24 {
        return None;
    }
    FixedOffset::east_opt(sign * (h * 3600 + m * 60))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// Broker fees are accrued and rounded in integer paise using ties-to-even.
pub struct FeeModel {
    pub bps: f64,
    pub flat_per_order: f64,
    pub min_per_order: f64,
}
fn round_half_even_i128(numerator: i128, denominator: i128) -> i128 {
    let q = numerator / denominator;
    let r = numerator % denominator;
    let twice = r.abs() * 2;
    if twice > denominator.abs() || (twice == denominator.abs() && q % 2 != 0) {
        q + numerator.signum() * denominator.signum()
    } else {
        q
    }
}
fn round_half_even_f64(value: f64) -> f64 {
    let floor = value.floor();
    let fraction = value - floor;
    if fraction < 0.5 {
        floor
    } else if fraction > 0.5 || (floor as i128).rem_euclid(2) != 0 {
        floor + 1.0
    } else {
        floor
    }
}
impl Default for FeeModel {
    fn default() -> Self {
        Self {
            bps: 0.0,
            flat_per_order: 0.0,
            min_per_order: 0.0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct SimBroker {
    pub symbols: Vec<Symbol>,
    pub latency: u64,
    pub slippage_bps: f64,
    /// Minimum permitted fill-price increment in paise (NSE equity default: 5).
    pub tick_size_paise: i64,
    pub tick_sizes_paise: BTreeMap<Symbol, i64>,
    pub partial_fills_pct: Option<f64>,
    pub fees: FeeModel,
    pending: BTreeMap<OrderId, PendingOrder>,
    events: Vec<OrderEvent>,
    ticks: BTreeMap<Symbol, u64>,
    seen_ids: std::collections::BTreeSet<OrderId>,
}
#[derive(Clone, Debug)]
struct PendingOrder {
    req: OrderRequest,
    remaining: u64,
    due: u64,
    acked: bool,
    notional_filled_paise: i128,
    fees_charged_paise: i64,
}
impl SimBroker {
    pub fn new(symbols: Vec<Symbol>) -> Self {
        Self {
            symbols,
            latency: 1,
            slippage_bps: 0.0,
            tick_size_paise: 5,
            tick_sizes_paise: BTreeMap::new(),
            partial_fills_pct: None,
            fees: FeeModel::default(),
            pending: BTreeMap::new(),
            events: Vec::new(),
            ticks: BTreeMap::new(),
            seen_ids: std::collections::BTreeSet::new(),
        }
    }
    pub fn on_candle_open(&mut self, candles: &BTreeMap<Symbol, Candle>) {
        for s in candles.keys() {
            *self.ticks.entry(s.clone()).or_default() += 1;
        }
        let mut ids: Vec<_> = self.pending.keys().cloned().collect();
        ids.sort();
        for id in ids {
            let Some(p) = self.pending.get_mut(&id) else {
                continue;
            };
            let Some(c) = candles.get(&p.req.symbol) else {
                continue;
            };
            let tick = *self.ticks.get(&p.req.symbol).unwrap_or(&0);
            if tick < p.due {
                continue;
            }
            if !p.acked {
                self.events.push(OrderEvent::Acked { id: id.clone() });
                p.acked = true;
            }
            let cap = self
                .partial_fills_pct
                .map(|pct| (c.volume * pct / 100.0).floor() as u64)
                .unwrap_or(p.remaining);
            let qty = p.remaining.min(cap);
            if qty == 0 {
                p.due = tick + 1;
                continue;
            }
            let raw_price = if p.req.side == Side::Buy {
                c.open * (1.0 + self.slippage_bps / 10000.0)
            } else {
                c.open * (1.0 - self.slippage_bps / 10000.0)
            };
            let tick_paise = self
                .tick_sizes_paise
                .get(&p.req.symbol)
                .copied()
                .unwrap_or(self.tick_size_paise)
                .max(1);
            let tick_units = raw_price * 100.0 / tick_paise as f64;
            let nearest = tick_units.round();
            let units = if (tick_units - nearest).abs() <= 1e-10 {
                nearest
            } else if p.req.side == Side::Buy {
                tick_units.ceil()
            } else {
                tick_units.floor()
            };
            let price_paise = (units as i64).saturating_mul(tick_paise);
            let price = crate::trader::paise_to_rupees(price_paise);
            p.notional_filled_paise += qty as i128 * price_paise as i128;
            let target_fee_paise = if self.fees.bps == 0.0
                && self.fees.flat_per_order == 0.0
                && self.fees.min_per_order == 0.0
            {
                0i64
            } else {
                // Convert the configured rate to integer hundredths of a bp
                // once; all notional multiplication and tie decisions below
                // are integer-only (denominator: 10,000 * 100).
                let bps_hundredths = round_half_even_f64(self.fees.bps * 100.0) as i128;
                let variable = round_half_even_i128(
                    p.notional_filled_paise * bps_hundredths,
                    10_000i128 * 100,
                );
                let flat = crate::trader::rupees_to_paise(self.fees.flat_per_order) as i128;
                let minimum = crate::trader::rupees_to_paise(self.fees.min_per_order) as i128;
                (variable + flat).max(minimum) as i64
            };
            let fees_paise = (target_fee_paise - p.fees_charged_paise).max(0);
            p.fees_charged_paise = target_fee_paise;
            let fees = crate::trader::paise_to_rupees(fees_paise);
            p.remaining -= qty;
            let done = p.remaining == 0;
            if done {
                self.events.push(OrderEvent::Filled {
                    id: id.clone(),
                    qty,
                    price,
                    fees,
                });
            } else {
                self.events.push(OrderEvent::PartialFill {
                    id: id.clone(),
                    qty,
                    price,
                    fees,
                });
                p.due = tick + 1;
            }
            if done {
                self.pending.remove(&id);
            }
        }
    }
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}
impl ExecutionPort for SimBroker {
    fn submit(&mut self, r: &OrderRequest) -> Result<(), PortError> {
        if r.qty == 0
            || !self.symbols.contains(&r.symbol)
            || self.seen_ids.contains(&r.client_order_id)
        {
            return Err(PortError::Message("invalid or duplicate order".into()));
        }
        let tick = *self.ticks.get(&r.symbol).unwrap_or(&0);
        self.pending.insert(
            r.client_order_id.clone(),
            PendingOrder {
                req: r.clone(),
                remaining: r.qty,
                due: tick + self.latency.max(1),
                acked: false,
                notional_filled_paise: 0,
                fees_charged_paise: 0,
            },
        );
        self.seen_ids.insert(r.client_order_id.clone());
        Ok(())
    }
    fn cancel(&mut self, id: OrderId) -> Result<(), PortError> {
        if self.pending.remove(&id).is_some() {
            self.events.push(OrderEvent::Cancelled { id });
            Ok(())
        } else {
            Err(PortError::UnknownOrder(id))
        }
    }
    fn drain_events(&mut self) -> Vec<OrderEvent> {
        self.events.sort_by(|a, b| event_id(a).cmp(event_id(b)));
        std::mem::take(&mut self.events)
    }
}
fn event_id(e: &OrderEvent) -> &str {
    match e {
        OrderEvent::Acked { id }
        | OrderEvent::PartialFill { id, .. }
        | OrderEvent::Filled { id, .. }
        | OrderEvent::Rejected { id, .. }
        | OrderEvent::Cancelled { id }
        | OrderEvent::Expired { id } => id,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunConfig {
    pub timeframe: Timeframe,
    pub range: DateRange,
    pub initial_cash: f64,
    pub warmup_bars: usize,
    pub latency: u64,
    pub slippage_bps: f64,
    pub fee_model: FeeModel,
    pub partial_fills_pct: Option<f64>,
    pub gap_policy: GapPolicy,
    pub debug_invariants: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fill {
    pub ts: i64,
    pub symbol: Symbol,
    pub order_id: OrderId,
    pub side: Side,
    pub qty: u64,
    pub price: f64,
    pub price_paise: i64,
    pub fees: f64,
    pub fees_paise: i64,
    pub reason: OrderReason,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EquityPoint {
    pub ts: i64,
    pub cash: f64,
    pub positions_value: f64,
    pub equity: f64,
    pub cash_paise: i64,
    pub positions_value_paise: i64,
    pub equity_paise: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenPosition {
    pub symbol: Symbol,
    pub qty: i64,
    pub avg_entry: f64,
    pub last_close: f64,
    pub unrealized: f64,
    pub avg_entry_paise: i64,
    pub last_close_paise: i64,
    pub unrealized_paise: i64,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Trade {
    pub symbol: Symbol,
    pub entry_ts: i64,
    pub exit_ts: i64,
    pub qty: u64,
    pub avg_entry: f64,
    pub avg_entry_paise: i64,
    pub avg_exit: f64,
    pub avg_exit_paise: i64,
    pub fees: f64,
    pub fees_paise: i64,
    pub net_pnl: f64,
    pub net_pnl_paise: i64,
    pub exit_reason: OrderReason,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SymbolEquityPoint {
    pub ts: i64,
    pub equity: f64,
    pub equity_paise: i64,
    pub position_qty: i64,
}
#[derive(Clone, Debug, Serialize)]
pub struct RunResult {
    pub fills: Vec<Fill>,
    pub equity: Vec<EquityPoint>,
    pub events: Vec<TraderEvent>,
    pub open_positions: Vec<OpenPosition>,
    pub manifest: serde_json::Value,
    pub source_stats: SourceStats,
    pub trades: Vec<Trade>,
    pub symbol_equity: BTreeMap<Symbol, Vec<SymbolEquityPoint>>,
}
#[derive(Debug, Error)]
pub enum SimError {
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    Engine(#[from] crate::engine::EngineError),
    #[error("{0}")]
    Other(String),
}
fn check_invariants(
    trader: &Trader,
    symbols: &[Symbol],
    fills: &[Fill],
    ts: i64,
) -> Result<(), SimError> {
    if trader.cash_paise() < 0 {
        return Err(SimError::Other(format!(
            "invariant: cash is negative at {ts}"
        )));
    }
    if trader.pending_order_count() == 0 && trader.reserved_cash_paise() != 0 {
        return Err(SimError::Other(format!(
            "invariant: reserved cash remains without pending orders at {ts}"
        )));
    }
    for symbol in symbols {
        let signed: i128 = fills
            .iter()
            .filter(|f| &f.symbol == symbol)
            .map(|f| {
                if f.side == Side::Buy {
                    f.qty as i128
                } else {
                    -(f.qty as i128)
                }
            })
            .sum();
        let actual = trader.position(symbol).map_or(0, |p| p.filled_qty) as i128;
        if signed != actual || actual < 0 {
            return Err(SimError::Other(format!(
                "invariant: fill ledger position mismatch for {symbol} at {ts}"
            )));
        }
    }
    Ok(())
}
/// Running per-symbol P&L state, folded forward one fill at a time by the sim
/// frame loop. `cash_flow_paise` is the net cash effect of this symbol's fills
/// (buys negative incl. fees, sells positive net of prorated fees) and `qty` is
/// the open quantity; a symbol's P&L at any mark is `cash_flow_paise + mark*qty`.
/// Replaces the former `symbol_pnl` replay, which re-scanned all fills per symbol
/// per frame (quadratic over a long run).
#[derive(Clone, Copy, Debug, Default)]
struct SymbolPnlAccum {
    qty: u64,
    cash_flow_paise: i128,
}
/// Stream a merged source through independent per-symbol engines, then process
/// broker events before each close frame in deterministic symbol order.
pub fn run(
    program: &Program,
    source: &dyn CandleSource,
    config: RunConfig,
    requested: Option<Vec<Symbol>>,
) -> Result<RunResult, SimError> {
    run_internal(program, source, config, requested, None)
}
/// Precompute each symbol's frames in parallel, then replay through the same
/// trader/broker timeline. This is the Stage-5 path and shares the streaming
/// accounting and event ordering implementation.
pub fn run_precomputed(
    program: &Program,
    source: &dyn CandleSource,
    config: RunConfig,
    requested: Option<Vec<Symbol>>,
) -> Result<RunResult, SimError> {
    let mut symbols = requested
        .clone()
        .unwrap_or_else(|| program.symbols().into_iter().map(str::to_owned).collect());
    symbols.sort();
    symbols.dedup();
    let stream = source.open(
        &symbols,
        config.timeframe,
        DateRange {
            from: None,
            to: config.range.to,
        },
    )?;
    let stream = limit_to_warmup_window(stream, config.range.from, config.warmup_bars);
    let mut series = BTreeMap::<Symbol, Vec<Candle>>::new();
    for item in stream {
        let (s, c) = item?;
        series.entry(s).or_default().push(c)
    }
    let computed: Vec<Result<(Symbol, Vec<LevelFrame>), crate::engine::EngineError>> = symbols
        .par_iter()
        .map(|s| {
            let candles = series.get(s).map(Vec::as_slice).unwrap_or(&[]);
            crate::engine::evaluate_series(program, s.clone(), candles)
                .map(|frames| (s.clone(), frames))
        })
        .collect();
    let mut frames = HashMap::<(Symbol, i64), LevelFrame>::new();
    for item in computed {
        let (s, values) = item?;
        for frame in values {
            frames.insert((s.clone(), frame.candle_ts), frame);
        }
    }
    run_internal(program, source, config, Some(symbols), Some(frames))
}
fn run_internal(
    program: &Program,
    source: &dyn CandleSource,
    config: RunConfig,
    requested: Option<Vec<Symbol>>,
    mut precomputed: Option<HashMap<(Symbol, i64), LevelFrame>>,
) -> Result<RunResult, SimError> {
    if !config.initial_cash.is_finite() || config.initial_cash <= 0.0 {
        return Err(SimError::Other(
            "initial cash must be finite and positive".into(),
        ));
    }
    if config.latency == 0 {
        return Err(SimError::Other(
            "latency must be at least one candle".into(),
        ));
    }
    if !config.slippage_bps.is_finite() || !(0.0..10000.0).contains(&config.slippage_bps) {
        return Err(SimError::Other(
            "slippage basis points must be in [0,10000)".into(),
        ));
    }
    if [
        config.fee_model.bps,
        config.fee_model.flat_per_order,
        config.fee_model.min_per_order,
    ]
    .iter()
    .any(|x| !x.is_finite() || *x < 0.0)
    {
        return Err(SimError::Other(
            "fee model values must be finite and non-negative".into(),
        ));
    }
    if config
        .partial_fills_pct
        .is_some_and(|p| !p.is_finite() || p <= 0.0 || p > 100.0)
    {
        return Err(SimError::Other(
            "partial fill percentage must be in (0,100]".into(),
        ));
    }
    if config
        .range
        .from
        .zip(config.range.to)
        .is_some_and(|(from, to)| from > to)
    {
        return Err(SimError::Other("--from must be at or before --to".into()));
    }
    let mut symbols =
        requested.unwrap_or_else(|| program.symbols().into_iter().map(str::to_owned).collect());
    symbols.sort();
    symbols.dedup();
    if symbols.is_empty() {
        return Err(SimError::Other("at least one symbol is required".into()));
    }
    let fp = source.fingerprint(&symbols, config.timeframe, config.range);
    if std::env::var_os("ALGOMLN_BENCHMARK").is_none() {
        eprintln!("source fingerprint: {}", fp.0);
    }
    let stream = source.open(
        &symbols,
        config.timeframe,
        DateRange {
            from: None,
            to: config.range.to,
        },
    )?;
    let stream = limit_to_warmup_window(stream, config.range.from, config.warmup_bars);
    let mut engines: BTreeMap<Symbol, Engine> = symbols
        .iter()
        .map(|s| Ok((s.clone(), Engine::new(program, s.clone())?)))
        .collect::<Result<_, crate::engine::EngineError>>()?;
    let mut trader = Trader::new(
        program,
        TraderConfig {
            initial_cash: config.initial_cash,
            instrument_rules: source.instrument_rules(&symbols),
        },
    )
    .map_err(|e| SimError::Other(format!("trader init: {e:?}")))?;
    let mut broker = SimBroker::new(symbols.clone());
    broker.latency = config.latency;
    broker.slippage_bps = config.slippage_bps;
    broker.tick_sizes_paise = source.tick_sizes_paise(&symbols);
    broker.fees = config.fee_model.clone();
    broker.partial_fills_pct = config.partial_fills_pct;
    let mut fills = Vec::new();
    let mut equity = Vec::new();
    let mut symbol_equity: BTreeMap<Symbol, Vec<SymbolEquityPoint>> =
        symbols.iter().map(|s| (s.clone(), Vec::new())).collect();
    let mut events = Vec::new();
    let mut last = BTreeMap::<Symbol, Candle>::new();
    let mut reqs = HashMap::<OrderId, OrderRequest>::new();
    let mut day = None;
    let mut current_ts = None;
    let mut candles = BTreeMap::<Symbol, Candle>::new();
    let mut observed = BTreeMap::<Symbol, usize>::new();
    // Incremental per-symbol P&L, folded forward as fills arrive instead of
    // replaying the whole fills vector per symbol per frame (was O(frames x fills)).
    let mut symbol_pnl_state = BTreeMap::<Symbol, SymbolPnlAccum>::new();
    let mut processed_fills = 0usize;
    let mut process = |ts: i64,
                       candles: &BTreeMap<Symbol, Candle>,
                       trader: &mut Trader,
                       broker: &mut SimBroker,
                       engines: &mut BTreeMap<Symbol, Engine>,
                       last: &mut BTreeMap<Symbol, Candle>,
                       fills: &mut Vec<Fill>,
                       events: &mut Vec<TraderEvent>,
                       reqs: &mut HashMap<OrderId, OrderRequest>,
                       equity: &mut Vec<EquityPoint>,
                       symbol_equity: &mut BTreeMap<Symbol, Vec<SymbolEquityPoint>>,
                       day: &mut Option<i64>,
                       observed: &mut BTreeMap<Symbol, usize>,
                       symbol_pnl_state: &mut BTreeMap<Symbol, SymbolPnlAccum>,
                       processed_fills: &mut usize|
     -> Result<(), SimError> {
        let d = ist_day(ts);
        if *day != Some(d) {
            trader.start_day(d);
            *day = Some(d);
            if (config.debug_invariants || cfg!(test))
                && (trader.realized_pnl_today_paise() != 0
                    || trader.realized_loss_today_paise() != 0
                    || trader.accepted_buys_today() != 0)
            {
                return Err(SimError::Other(format!(
                    "invariant: daily counters did not reset for IST day {d}"
                )));
            }
        }
        broker.on_candle_open(candles);
        let order_events = broker.drain_events();
        for ev in order_events {
            if let Some(id) = match &ev {
                OrderEvent::PartialFill { id, .. } | OrderEvent::Filled { id, .. } => Some(id),
                _ => None,
            } {
                if let Some(req) = reqs.get(id) {
                    if ts <= req.decided_at && (config.debug_invariants || cfg!(test)) {
                        return Err(SimError::Other(format!(
                            "invariant: order {id} filled before its next candle open"
                        )));
                    }
                }
            }
            if let Some(fill) = to_fill(ts, &ev, reqs) {
                fills.push(fill)
            }
            events.extend(trader.on_order_event(ev));
        }
        for (s, c) in candles {
            last.insert(s.clone(), *c);
            let frame = if let Some(frames) = precomputed.as_mut() {
                frames.remove(&(s.clone(), c.timestamp)).ok_or_else(|| {
                    SimError::Other(format!(
                        "precomputed frame missing for {s} at {}",
                        c.timestamp
                    ))
                })?
            } else {
                engines
                    .get_mut(s)
                    .expect("configured symbol")
                    .on_candle(c)?
            };
            let count = observed.entry(s.clone()).or_default();
            let mode = if let Some(from) = config.range.from {
                if ts < from {
                    Mode::Observe
                } else {
                    Mode::Act
                }
            } else if *count < config.warmup_bars {
                *count += 1;
                Mode::Observe
            } else {
                Mode::Act
            };
            let out = trader.on_frame(&frame, c, mode);
            events.extend(out.events.clone());
            for id in out.cancels {
                broker
                    .cancel(id)
                    .map_err(|e| SimError::Other(format!("broker cancel: {e:?}")))?;
            }
            for req in &out.orders {
                broker
                    .submit(req)
                    .map_err(|e| SimError::Other(format!("broker submit: {e:?}")))?;
                if reqs
                    .insert(req.client_order_id.clone(), req.clone())
                    .is_some()
                {
                    return Err(SimError::Other(format!(
                        "invariant: duplicate order id {}",
                        req.client_order_id
                    )));
                }
            }
        }
        let mut pos_value_paise = 0i128;
        for s in &symbols {
            if let (Some(p), Some(c)) = (trader.position(s), last.get(s)) {
                let paise = crate::trader::rupees_to_paise(c.close);
                pos_value_paise += p.filled_qty as i128 * paise as i128;
            }
        }
        let cash_paise = trader.cash_paise();
        let positions_value_paise =
            pos_value_paise.clamp(i64::MIN as i128, i64::MAX as i128) as i64;
        let equity_paise = cash_paise.saturating_add(positions_value_paise);
        equity.push(EquityPoint {
            ts,
            cash: crate::trader::paise_to_rupees(cash_paise),
            positions_value: crate::trader::paise_to_rupees(positions_value_paise),
            equity: crate::trader::paise_to_rupees(equity_paise),
            cash_paise,
            positions_value_paise,
            equity_paise,
        });
        // Fold the fills added since the last frame into the per-symbol P&L
        // accumulators once, then read each symbol's running total below. This
        // replaces the previous full replay of `fills` per symbol per frame.
        for f in &fills[*processed_fills..] {
            let acc = symbol_pnl_state.entry(f.symbol.clone()).or_default();
            match f.side {
                Side::Buy => {
                    acc.qty += f.qty;
                    acc.cash_flow_paise -=
                        f.price_paise as i128 * f.qty as i128 + f.fees_paise as i128;
                }
                Side::Sell => {
                    let q = f.qty.min(acc.qty);
                    if q > 0 {
                        acc.qty -= q;
                        acc.cash_flow_paise += f.price_paise as i128 * q as i128
                            - round_half_even_i128(
                                f.fees_paise as i128 * q as i128,
                                f.qty.max(1) as i128,
                            );
                    }
                }
            }
        }
        *processed_fills = fills.len();
        let initial_cash_paise = crate::trader::rupees_to_paise(config.initial_cash);
        let allocation_base = initial_cash_paise / symbols.len().max(1) as i64;
        let allocation_remainder = initial_cash_paise.rem_euclid(symbols.len().max(1) as i64);
        for (symbol_index, symbol) in symbols.iter().enumerate() {
            let mark = last
                .get(symbol)
                .map_or(0, |c| crate::trader::rupees_to_paise(c.close));
            // Running P&L against the current mark: identical to the old
            // `symbol_pnl` replay, but O(1) per symbol per frame.
            let (pnl, qty) = symbol_pnl_state.get(symbol).map_or((0i64, 0i64), |acc| {
                let pnl_paise = acc.cash_flow_paise + mark as i128 * acc.qty as i128;
                (
                    pnl_paise.clamp(i64::MIN as i128, i64::MAX as i128) as i64,
                    acc.qty as i64,
                )
            });
            // `allocation_base` is a static even split of initial cash used only as
            // the per-symbol baseline; the series tracks relative P&L and does not
            // attribute pooled or reserved cash per symbol, so it need not reconcile
            // exactly to portfolio equity while cash is reserved for pending orders.
            let allocation_paise =
                allocation_base + i64::from((symbol_index as i64) < allocation_remainder);
            let symbol_equity_paise = allocation_paise.saturating_add(pnl);
            symbol_equity
                .entry(symbol.clone())
                .or_default()
                .push(SymbolEquityPoint {
                    ts,
                    equity: crate::trader::paise_to_rupees(symbol_equity_paise),
                    equity_paise: symbol_equity_paise,
                    position_qty: qty,
                });
        }
        if config.debug_invariants || cfg!(test) {
            check_invariants(trader, &symbols, fills, ts)?;
        }
        Ok(())
    };
    for item in stream {
        let (s, c) = item?;
        if current_ts.is_some_and(|t| t != c.timestamp) {
            process(
                current_ts.unwrap(),
                &candles,
                &mut trader,
                &mut broker,
                &mut engines,
                &mut last,
                &mut fills,
                &mut events,
                &mut reqs,
                &mut equity,
                &mut symbol_equity,
                &mut day,
                &mut observed,
                &mut symbol_pnl_state,
                &mut processed_fills,
            )?;
            candles.clear();
        }
        current_ts = Some(c.timestamp);
        if candles.contains_key(&s) {
            return Err(SimError::Other(format!(
                "duplicate candle for symbol {s:?} at timestamp {}",
                c.timestamp
            )));
        }
        candles.insert(s, c);
    }
    if let Some(ts) = current_ts {
        process(
            ts,
            &candles,
            &mut trader,
            &mut broker,
            &mut engines,
            &mut last,
            &mut fills,
            &mut events,
            &mut reqs,
            &mut equity,
            &mut symbol_equity,
            &mut day,
            &mut observed,
            &mut symbol_pnl_state,
            &mut processed_fills,
        )?;
    }
    if config.debug_invariants || cfg!(test) {
        if let Some(last_point) = equity.last() {
            let mut qty = BTreeMap::<Symbol, u64>::new();
            let mut cash_paise = crate::trader::rupees_to_paise(config.initial_cash) as i128;
            for fill in &fills {
                let q = qty.entry(fill.symbol.clone()).or_default();
                match fill.side {
                    Side::Buy => {
                        *q += fill.qty;
                        cash_paise -=
                            fill.price_paise as i128 * fill.qty as i128 + fill.fees_paise as i128;
                    }
                    Side::Sell => {
                        if fill.qty > *q {
                            return Err(SimError::Other(format!(
                                "invariant: sell fill exceeds position for {}",
                                fill.symbol
                            )));
                        }
                        *q -= fill.qty;
                        cash_paise +=
                            fill.price_paise as i128 * fill.qty as i128 - fill.fees_paise as i128;
                    }
                }
            }
            let positions_value_paise: i128 = symbols
                .iter()
                .map(|s| {
                    let q = *qty.get(s).unwrap_or(&0);
                    let mark = last
                        .get(s)
                        .map_or(0, |x| crate::trader::rupees_to_paise(x.close));
                    mark as i128 * q as i128
                })
                .sum();
            let recomputed_equity = cash_paise + positions_value_paise;
            if cash_paise != last_point.cash_paise as i128
                || positions_value_paise != last_point.positions_value_paise as i128
                || recomputed_equity != last_point.equity_paise as i128
            {
                return Err(SimError::Other(
                    "invariant: paise cash and positions recomputed from fills do not exactly reconcile to final equity".into(),
                ));
            }
        }
    }
    let open_positions = symbols
        .iter()
        .filter_map(|s| {
            let p = trader.position(s)?;
            if p.filled_qty == 0 {
                return None;
            }
            let c = last.get(s)?;
            let last_close_paise = crate::trader::rupees_to_paise(c.close);
            let unrealized_paise =
                (last_close_paise - p.avg_entry_paise).saturating_mul(p.filled_qty);
            Some(OpenPosition {
                symbol: s.clone(),
                qty: p.filled_qty,
                avg_entry: crate::trader::paise_to_rupees(p.avg_entry_paise),
                last_close: crate::trader::paise_to_rupees(last_close_paise),
                unrealized: crate::trader::paise_to_rupees(unrealized_paise),
                avg_entry_paise: p.avg_entry_paise,
                last_close_paise,
                unrealized_paise,
            })
        })
        .collect();
    let mut ph = 0xcbf29ce484222325u64;
    for b in format!("{:?}", program.strategy()).bytes() {
        ph ^= b as u64;
        ph = ph.wrapping_mul(0x100000001b3);
    }
    let source_stats = source.stats();
    let commit = option_env!("GIT_COMMIT").map(str::to_owned).or_else(|| {
        std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    });
    let manifest = serde_json::json!({"program_hash":format!("fnv1a64:{ph:016x}"),"source_fingerprint":fp,"symbols":symbols,"timeframe":config.timeframe,"range":config.range,"config":config,"tool_version":env!("CARGO_PKG_VERSION"),"git_commit":commit,"source_stats":source_stats});
    if std::env::var_os("ALGOMLN_BENCHMARK").is_none() {
        eprintln!("source fingerprint: {}", fp.0);
    }
    let trades = derive_trades(&fills);
    Ok(RunResult {
        fills,
        equity,
        events,
        open_positions,
        manifest,
        source_stats,
        trades,
        symbol_equity,
    })
}
fn ist_day(ts: i64) -> i64 {
    ts + 19800 - (ts + 19800).rem_euclid(86400)
}
fn to_fill(ts: i64, e: &OrderEvent, reqs: &HashMap<OrderId, OrderRequest>) -> Option<Fill> {
    let (id, qty, price, fees) = match e {
        OrderEvent::Filled {
            id,
            qty,
            price,
            fees,
        }
        | OrderEvent::PartialFill {
            id,
            qty,
            price,
            fees,
        } => (id, *qty, *price, *fees),
        _ => return None,
    };
    let r = reqs.get(id)?;
    let price = crate::trader::paise_to_rupees(crate::trader::rupees_to_paise(price));
    let fees = crate::trader::paise_to_rupees(crate::trader::rupees_to_paise(fees));
    Some(Fill {
        ts,
        symbol: r.symbol.clone(),
        order_id: id.clone(),
        side: r.side,
        qty,
        price,
        price_paise: crate::trader::rupees_to_paise(price),
        fees,
        fees_paise: crate::trader::rupees_to_paise(fees),
        reason: r.reason.clone(),
    })
}
fn derive_trades(fills: &[Fill]) -> Vec<Trade> {
    #[derive(Default)]
    struct Lot {
        qty: u64,
        cost_paise: i128,
        entry_fees_paise: i128,
        entry_ts: i64,
        closed_qty: u64,
        closed_entry_cost_paise: i128,
        closed_exit_value_paise: i128,
        closed_fees_paise: i128,
        closed_pnl_paise: i128,
    }
    let mut lots = BTreeMap::<Symbol, Lot>::new();
    let mut trades = Vec::new();
    for f in fills {
        let lot = lots.entry(f.symbol.clone()).or_default();
        match f.side {
            Side::Buy => {
                if lot.qty == 0 {
                    lot.entry_ts = f.ts;
                }
                lot.qty += f.qty;
                lot.cost_paise += f.price_paise as i128 * f.qty as i128;
                lot.entry_fees_paise += f.fees_paise as i128;
            }
            Side::Sell => {
                if lot.qty == 0 {
                    continue;
                }
                let q = f.qty.min(lot.qty);
                let entry_cost = round_half_even_i128(lot.cost_paise * q as i128, lot.qty as i128);
                let entry_fee =
                    round_half_even_i128(lot.entry_fees_paise * q as i128, lot.qty as i128);
                let exit_fee =
                    round_half_even_i128(f.fees_paise as i128 * q as i128, f.qty.max(1) as i128);
                let exit_value = f.price_paise as i128 * q as i128;
                lot.closed_qty += q;
                lot.closed_entry_cost_paise += entry_cost;
                lot.closed_exit_value_paise += exit_value;
                lot.closed_fees_paise += entry_fee + exit_fee;
                lot.closed_pnl_paise += exit_value - entry_cost - entry_fee - exit_fee;
                lot.qty -= q;
                lot.cost_paise -= entry_cost;
                lot.entry_fees_paise -= entry_fee;
                if lot.qty == 0 && lot.closed_qty > 0 {
                    trades.push(Trade {
                        symbol: f.symbol.clone(),
                        entry_ts: lot.entry_ts,
                        exit_ts: f.ts,
                        qty: lot.closed_qty,
                        avg_entry: crate::trader::paise_to_rupees(round_half_even_i128(
                            lot.closed_entry_cost_paise,
                            lot.closed_qty as i128,
                        ) as i64),
                        avg_entry_paise: round_half_even_i128(
                            lot.closed_entry_cost_paise,
                            lot.closed_qty as i128,
                        ) as i64,
                        avg_exit: crate::trader::paise_to_rupees(round_half_even_i128(
                            lot.closed_exit_value_paise,
                            lot.closed_qty as i128,
                        ) as i64),
                        avg_exit_paise: round_half_even_i128(
                            lot.closed_exit_value_paise,
                            lot.closed_qty as i128,
                        ) as i64,
                        fees: crate::trader::paise_to_rupees(lot.closed_fees_paise as i64),
                        fees_paise: lot.closed_fees_paise as i64,
                        net_pnl: crate::trader::paise_to_rupees(lot.closed_pnl_paise as i64),
                        net_pnl_paise: lot.closed_pnl_paise as i64,
                        exit_reason: f.reason.clone(),
                    });
                    *lot = Lot::default();
                }
            }
        }
    }
    trades.sort_by(|a, b| (a.exit_ts, &a.symbol).cmp(&(b.exit_ts, &b.symbol)));
    trades
}

#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::*;
    use crate::trader::{ExecutionPort, OrderRequest, ScriptedPort};

    struct MemorySource(Vec<(Symbol, Candle)>);
    impl CandleSource for MemorySource {
        fn open(
            &self,
            symbols: &[Symbol],
            _tf: Timeframe,
            range: DateRange,
        ) -> Result<CandleStream, SourceError> {
            let values: Vec<_> = self
                .0
                .iter()
                .filter(|(s, c)| {
                    symbols.contains(s)
                        && range.from.map_or(true, |x| c.timestamp >= x)
                        && range.to.map_or(true, |x| c.timestamp <= x)
                })
                .cloned()
                .map(Ok)
                .collect();
            Ok(Box::new(values.into_iter()))
        }
        fn fingerprint(&self, _: &[Symbol], _: Timeframe, _: DateRange) -> Fingerprint {
            Fingerprint("memory-fixture".into())
        }
    }
    fn candle(ts: i64, open: f64, close: f64) -> Candle {
        Candle {
            timestamp: ts,
            open,
            high: open.max(close),
            low: open.min(close),
            close,
            volume: 100.0,
        }
    }
    fn config() -> RunConfig {
        RunConfig {
            timeframe: Timeframe::M1,
            range: DateRange::default(),
            initial_cash: 1000.0,
            warmup_bars: 0,
            latency: 1,
            slippage_bps: 0.0,
            fee_model: FeeModel::default(),
            partial_fills_pct: None,
            gap_policy: GapPolicy::None,
            debug_invariants: true,
        }
    }

    #[test]
    fn broker_acks_and_fills_at_next_available_open() {
        let mut b = SimBroker::new(vec!["ABC".into()]);
        let req = OrderRequest {
            client_order_id: "o1".into(),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 2,
            reason: OrderReason::Rule("rule_1".into()),
            ref_price: 10.0,
            decided_at: 1,
        };
        b.submit(&req).unwrap();
        b.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 12.0, 12.0))]));
        let ev = b.drain_events();
        assert!(matches!(ev[0], OrderEvent::Acked { .. }));
        assert!(matches!(
            ev[1],
            OrderEvent::Filled {
                price: 12.0,
                qty: 2,
                ..
            }
        ));
    }

    #[test]
    fn fee_rounding_is_half_even_in_integer_paise() {
        assert_eq!(round_half_even_i128(1, 2), 0);
        assert_eq!(round_half_even_i128(3, 2), 2);
        assert_eq!(round_half_even_i128(5, 2), 2);
        assert_eq!(round_half_even_i128(7, 2), 4);

        let mut even_broker = SimBroker::new(vec!["ABC".into()]);
        even_broker.fees.bps = 5000.0; // 1 paisa notional -> exactly 0.5 paisa
        even_broker.tick_size_paise = 1;
        even_broker
            .submit(&OrderRequest {
                client_order_id: "even-half".into(),
                symbol: "ABC".into(),
                side: Side::Buy,
                qty: 1,
                reason: OrderReason::Rule("r".into()),
                ref_price: 0.01,
                decided_at: 1,
            })
            .unwrap();
        even_broker.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 0.01, 0.01))]));
        assert!(matches!(
            even_broker.drain_events().as_slice(),
            [
                OrderEvent::Acked { .. },
                OrderEvent::Filled { fees: 0.0, .. }
            ]
        ));

        let mut broker = SimBroker::new(vec!["ABC".into()]);
        broker.fees.bps = 5000.0;
        broker.tick_size_paise = 1;
        broker
            .submit(&OrderRequest {
                client_order_id: "half-even-fee".into(),
                symbol: "ABC".into(),
                side: Side::Buy,
                qty: 3,
                reason: OrderReason::Rule("r".into()),
                ref_price: 0.01,
                decided_at: 1,
            })
            .unwrap();
        broker.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 0.01, 0.01))]));
        assert!(matches!(
            broker.drain_events().as_slice(),
            [
                OrderEvent::Acked { .. },
                OrderEvent::Filled { fees: 0.02, .. }
            ]
        ));
    }

    #[test]
    fn simulated_slippage_rounds_adversely_to_tick() {
        let mut buy = SimBroker::new(vec!["ABC".into()]);
        buy.slippage_bps = 1.0;
        buy.submit(&OrderRequest {
            client_order_id: "buy-tick".into(),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 1,
            reason: OrderReason::Rule("r".into()),
            ref_price: 10.0,
            decided_at: 1,
        })
        .unwrap();
        buy.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 10.01, 10.01))]));
        let events = buy.drain_events();
        let OrderEvent::Filled { price, .. } = events[1] else {
            panic!("expected buy fill")
        };
        assert_eq!(crate::trader::rupees_to_paise(price), 1005);

        let mut sell = SimBroker::new(vec!["ABC".into()]);
        sell.slippage_bps = 1.0;
        sell.submit(&OrderRequest {
            client_order_id: "sell-tick".into(),
            symbol: "ABC".into(),
            side: Side::Sell,
            qty: 1,
            reason: OrderReason::Rule("r".into()),
            ref_price: 10.0,
            decided_at: 1,
        })
        .unwrap();
        sell.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 10.04, 10.04))]));
        let events = sell.drain_events();
        let OrderEvent::Filled { price, .. } = events[1] else {
            panic!("expected sell fill")
        };
        assert_eq!(crate::trader::rupees_to_paise(price), 1000);
    }

    #[test]
    fn runner_is_deterministic_and_executes_next_open() {
        let p = crate::compile("TRADE_IN ABC\nWHEN close > 10\nBUY 1\nWHEN close < 9\nSELL ALL\n")
            .unwrap();
        let src = MemorySource(vec![
            ("ABC".into(), candle(1, 10.0, 11.0)),
            ("ABC".into(), candle(2, 12.0, 8.0)),
            ("ABC".into(), candle(3, 7.0, 7.0)),
        ]);
        let a = run(&p, &src, config(), None).unwrap();
        let b = run(&p, &src, config(), None).unwrap();
        let c = run_precomputed(&p, &src, config(), None).unwrap();
        assert_eq!(a.fills, b.fills);
        assert_eq!(a.equity, b.equity);
        assert_eq!(a.fills, c.fills);
        assert_eq!(a.equity, c.equity);
        assert_eq!(a.trades, c.trades);
        assert_eq!(a.fills.len(), 2);
        assert_eq!(a.fills[0].price, 12.0);
        assert_eq!(a.fills[1].price, 7.0);
        assert_eq!(a.trades.len(), 1);
        assert_eq!(a.trades[0].net_pnl, -5.0);
        let root = std::env::temp_dir().join(format!("algomln-result-{}", std::process::id()));
        let left = root.join("left");
        let right = root.join("right");
        let _ = std::fs::create_dir_all(&root);
        write_results(&left, &a).unwrap();
        write_results(&right, &b).unwrap();
        for name in [
            "fills.csv",
            "trades.csv",
            "equity.csv",
            "metrics.json",
            "manifest.json",
            "open_positions.json",
            "events.json",
        ] {
            assert_eq!(
                std::fs::read(left.join(name)).unwrap(),
                std::fs::read(right.join(name)).unwrap(),
                "{name}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stop_gap_uses_next_open_not_stop_price() {
        let p = crate::compile("TRADE_IN ABC\nSTOP_LOSS 10%\nWHEN close > 0\nBUY 1\n").unwrap();
        let src = MemorySource(vec![
            ("ABC".into(), candle(1, 100.0, 100.0)),
            ("ABC".into(), candle(2, 100.0, 89.0)),
            ("ABC".into(), candle(3, 80.0, 80.0)),
        ]);
        let r = run(&p, &src, config(), None).unwrap();
        assert_eq!(r.fills.len(), 2);
        assert_eq!(r.fills[1].price, 80.0);
        assert_eq!(r.fills[1].reason, OrderReason::StopLoss);
    }

    #[test]
    fn pending_order_waits_until_its_symbol_has_a_candle() {
        let p = crate::compile("TRADE_IN A,B\nWHEN close > 10\nBUY 1\n").unwrap();
        let src = MemorySource(vec![
            ("B".into(), candle(1, 10.0, 11.0)),
            ("A".into(), candle(2, 1.0, 1.0)),
            ("B".into(), candle(3, 12.0, 12.0)),
        ]);
        let r = run(&p, &src, config(), Some(vec!["B".into()])).unwrap();
        assert_eq!(r.fills.len(), 1);
        assert_eq!(r.fills[0].ts, 3);
        assert_eq!(r.fills[0].price, 12.0);
    }

    #[test]
    fn extra_inactive_symbol_does_not_change_single_symbol_fills() {
        let p = crate::compile("TRADE_IN ABC,XYZ\nWHEN close > 100\nBUY 1\n").unwrap();
        let src = MemorySource(vec![
            ("ABC".into(), candle(1, 100.0, 101.0)),
            ("XYZ".into(), candle(1, 1.0, 1.0)),
            ("ABC".into(), candle(2, 102.0, 102.0)),
            ("XYZ".into(), candle(2, 1.0, 1.0)),
        ]);
        let one = run(&p, &src, config(), Some(vec!["ABC".into()])).unwrap();
        let both = run(&p, &src, config(), None).unwrap();
        assert_eq!(one.fills, both.fills);
    }

    #[test]
    fn synthetic_generator_is_seeded_and_contains_requested_scenarios() {
        let symbols = vec!["AAA".to_owned(), "BBB".to_owned()];
        let first = seeded_synthetic_candles(42, &symbols, 4);
        assert_eq!(first, seeded_synthetic_candles(42, &symbols, 4));
        assert_ne!(first, seeded_synthetic_candles(43, &symbols, 4));
        assert!(first.iter().any(|(_, c)| c.volume == 0.0));
        assert!(first.iter().any(|(_, c)| c.high - c.low > 10.0));
        assert!(first
            .windows(2)
            .any(|w| w[1].1.timestamp - w[0].1.timestamp > 60));
        assert!(first.windows(2).any(|w| w[0].0 == "AAA" && w[1].0 == "BBB"));
    }

    #[test]
    fn file_merge_retains_only_one_decoded_candle_per_symbol() {
        let base = std::env::temp_dir().join(format!("algomln-bounded-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        for symbol in ["A", "B", "C"] {
            let mut csv = String::from("ts,open,high,low,close,volume\n");
            for ts in 1..=20_000 {
                csv.push_str(&format!("{ts},1,1,1,1,1\n"));
            }
            std::fs::write(base.join(format!("{symbol}.csv")), csv).unwrap();
        }
        let source = FileSource::new(&base);
        let stream = source
            .open(
                &["A".into(), "B".into(), "C".into()],
                Timeframe::M1,
                DateRange::default(),
            )
            .unwrap();
        // FileSource's merge owns only one cursor head per file; consuming the
        // entire stream also exercises row-at-a-time CSV decoding.
        assert_eq!(stream.count(), 60_000);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn scripted_port_and_sim_broker_latency_one_emit_identical_trade() {
        let req = OrderRequest {
            client_order_id: "trade-1".into(),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 2,
            reason: OrderReason::Rule("entry".into()),
            ref_price: 10.0,
            decided_at: 1,
        };
        let mut scripted = ScriptedPort::default();
        scripted.submit(&req).unwrap();
        scripted.push(OrderEvent::Acked {
            id: req.client_order_id.clone(),
        });
        scripted.push(OrderEvent::Filled {
            id: req.client_order_id.clone(),
            qty: 2,
            price: 12.0,
            fees: 0.0,
        });

        let mut broker = SimBroker::new(vec!["ABC".into()]);
        broker.latency = 1;
        broker.submit(&req).unwrap();
        broker.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 12.0, 12.0))]));
        assert_eq!(scripted.drain_events(), broker.drain_events());
    }

    #[test]
    fn pending_order_carries_across_day_rollover_and_counters_reset() {
        let program = crate::compile("TRADE_IN ABC\nWHEN close > 0\nBUY 1\n").unwrap();
        let day1 = chrono::NaiveDate::from_ymd_opt(2024, 5, 6).unwrap();
        let day2 = chrono::NaiveDate::from_ymd_opt(2024, 5, 7).unwrap();
        let ts1 = day1.and_hms_opt(10, 0, 0).unwrap().and_utc().timestamp();
        let ts2 = day2.and_hms_opt(10, 0, 0).unwrap().and_utc().timestamp();
        let source = MemorySource(vec![
            ("ABC".into(), candle(ts1, 10.0, 10.0)),
            ("ABC".into(), candle(ts2, 11.0, 11.0)),
        ]);
        let result = run(&program, &source, config(), None).unwrap();
        assert_eq!(result.fills.len(), 1);
        assert_eq!(result.fills[0].ts, ts2);
        assert_eq!(result.fills[0].price, 11.0);
    }

    #[test]
    fn orchestrated_daily_loss_flattens_cancels_entries_and_reopens_next_day() {
        let source_text =
            "TRADE_IN ABC\nRISK MAX_DAILY_LOSS 0.1%\nWHEN close > 0\nBUY 2\nWHEN close < 95\nSELL 1\n";
        let program = crate::compile(source_text).unwrap();
        let tz = FixedOffset::east_opt(19800).unwrap();
        let d1 = chrono::NaiveDate::from_ymd_opt(2024, 5, 6).unwrap();
        let d2 = chrono::NaiveDate::from_ymd_opt(2024, 5, 7).unwrap();
        let ts = |d: chrono::NaiveDate, h, m| {
            tz.from_local_datetime(&d.and_hms_opt(h, m, 0).unwrap())
                .single()
                .unwrap()
                .timestamp()
        };
        let source = MemorySource(vec![
            ("ABC".into(), candle(ts(d1, 9, 15), 100.0, 100.0)), // signals buy
            ("ABC".into(), candle(ts(d1, 9, 16), 100.0, 100.0)), // buy fill
            ("ABC".into(), candle(ts(d1, 9, 17), 90.0, 90.0)),   // losing exit signals
            ("ABC".into(), candle(ts(d1, 9, 18), 90.0, 90.0)),   // loss is realized
            ("ABC".into(), candle(ts(d1, 9, 19), 90.0, 90.0)),   // flatten after breach
            ("ABC".into(), candle(ts(d1, 9, 20), 90.0, 90.0)),   // flatten fill
            ("ABC".into(), candle(ts(d2, 9, 15), 90.0, 90.0)),   // next-day signal
            ("ABC".into(), candle(ts(d2, 9, 16), 90.0, 90.0)),   // next-day fill
        ]);
        let mut cfg = config();
        cfg.initial_cash = 10_000.0;
        let result = run(&program, &source, cfg, None).unwrap();
        assert_eq!(result.fills.len(), 3);
        assert_eq!(result.fills[0].side, Side::Buy);
        assert_eq!(result.fills[1].side, Side::Sell);
        assert_eq!(result.fills[1].reason, OrderReason::Rule("rule_2".into()));
        assert_eq!(result.fills[2].side, Side::Sell);
        assert_eq!(result.fills[2].reason, OrderReason::DailyLossFlatten);
        assert_eq!(result.open_positions.iter().map(|p| p.qty).sum::<i64>(), 0);
        assert!(result
            .events
            .iter()
            .any(|event| matches!(event, TraderEvent::DailyLossBreached)));
    }

    #[test]
    fn date_warmup_keeps_only_the_requested_tail_per_symbol() {
        let source: Vec<Result<(Symbol, Candle), SourceError>> = (1..=5)
            .map(|ts| Ok(("A".into(), candle(ts, 1.0, 1.0))))
            .collect();
        let from = 4;
        let out: Vec<_> = limit_to_warmup_window(Box::new(source.into_iter()), Some(from), 2)
            .map(Result::unwrap)
            .map(|(_, c)| c.timestamp)
            .collect();
        assert_eq!(out, vec![2, 3, 4, 5]);
    }

    #[test]
    fn skip_bad_rows_are_counted_and_valid_rows_survive() {
        let base = std::env::temp_dir().join(format!("algomln-skip-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        std::fs::write(
            base.join("A.csv"),
            "ts,open,high,low,close,volume\n1,1,1,1,1,1\nbad,1,1,1,1,1\n2,2,2,2,2,1\n",
        )
        .unwrap();
        let mut src = FileSource::new(&base);
        src.skip_bad = true;
        let stream = src
            .open(&["A".into()], Timeframe::M1, DateRange::default())
            .unwrap();
        let rows: Vec<_> = stream.map(Result::unwrap).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(src.stats().skipped_bad["A"], 1);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn golden_strategy_and_csv_produce_expected_round_trip_file() {
        let strategy = include_str!("../tests/golden/simulation.algomln");
        let p = crate::compile(strategy).unwrap();
        let src = FileSource::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden"));
        let mut cfg = config();
        cfg.initial_cash = 1000.0;
        let r = run(&p, &src, cfg, None).unwrap();
        let dir = std::env::temp_dir().join(format!("algomln-golden-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        write_results(&dir, &r).unwrap();
        let actual = std::fs::read_to_string(dir.join("trades.csv")).unwrap();
        assert_eq!(
            actual,
            include_str!("../tests/golden/simulation.trades.csv")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_stream_merges_by_timestamp_then_symbol() {
        let base = std::env::temp_dir().join(format!("algomln-sim-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        std::fs::write(
            base.join("A.csv"),
            "ts,open,high,low,close,volume\n1,1,1,1,1,1\n",
        )
        .unwrap();
        std::fs::write(
            base.join("B.csv"),
            "ts,open,high,low,close,volume\n1,1,1,1,1,1\n",
        )
        .unwrap();
        let src = FileSource::new(&base);
        let mut stream = src
            .open(
                &["B".into(), "A".into()],
                Timeframe::M1,
                DateRange::default(),
            )
            .unwrap();
        let a = stream.next().unwrap().unwrap();
        let b = stream.next().unwrap().unwrap();
        assert_eq!((a.0, a.1.timestamp), ("A".into(), 1));
        assert_eq!((b.0, b.1.timestamp), ("B".into(), 1));
        drop(stream);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn file_source_fills_flat_only_inside_same_ist_session() {
        let base = std::env::temp_dir().join(format!("algomln-gaps-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        let tz = FixedOffset::east_opt(19800).unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2024, 5, 6).unwrap();
        let t1 = tz
            .from_local_datetime(&day.and_hms_opt(9, 15, 0).unwrap())
            .single()
            .unwrap()
            .timestamp();
        let t3 = tz
            .from_local_datetime(&day.and_hms_opt(9, 17, 0).unwrap())
            .single()
            .unwrap()
            .timestamp();
        std::fs::write(
            base.join("ABC.csv"),
            format!("ts,open,high,low,close,volume\n{t1},10,10,10,10,1\n{t3},12,12,12,12,1\n"),
        )
        .unwrap();
        let mut src = FileSource::new(&base);
        src.gap_policy = GapPolicy::FillFlatInSession;
        let stream = src
            .open(&["ABC".into()], Timeframe::M1, DateRange::default())
            .unwrap();
        let got: Vec<_> = stream.map(Result::unwrap).map(|(_, c)| c).collect();
        assert_eq!(got.len(), 3);
        assert_eq!(got[1].timestamp, t1 + 60);
        assert_eq!(got[1].close, 10.0);
        assert_eq!(got[1].volume, 0.0);
        assert_eq!(src.stats().missing_candles["ABC"], 1);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn daily_series_is_never_gap_filled() {
        let base = std::env::temp_dir().join(format!("algomln-d1gap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        let tz = FixedOffset::east_opt(19800).unwrap();
        let ts_at = |y, m, d| {
            let day = chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
            tz.from_local_datetime(&day.and_hms_opt(9, 15, 0).unwrap())
                .single()
                .unwrap()
                .timestamp()
        };
        // Two daily bars three calendar days apart (a weekend gap between them).
        let t1 = ts_at(2024, 5, 3); // Fri
        let t2 = ts_at(2024, 5, 6); // Mon
        std::fs::write(
            base.join("ABC.csv"),
            format!("ts,open,high,low,close,volume\n{t1},10,10,10,10,1\n{t2},12,12,12,12,1\n"),
        )
        .unwrap();
        let mut src = FileSource::new(&base);
        src.gap_policy = GapPolicy::FillFlatInSession;
        let stream = src
            .open(&["ABC".into()], Timeframe::D1, DateRange::default())
            .unwrap();
        let got: Vec<_> = stream.map(Result::unwrap).map(|(_, c)| c).collect();
        // Daily gaps are non-trading days and must never be filled or counted.
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].timestamp, t1);
        assert_eq!(got[1].timestamp, t2);
        assert!(src.stats().missing_candles.get("ABC").is_none());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn parquet_reader_streams_validated_ohlcv_rows() {
        use arrow_array::{Float64Array, Int64Array, RecordBatch};
        use arrow_schema::{DataType, Field, Schema};
        use parquet::arrow::ArrowWriter;
        use std::sync::Arc;
        let base = std::env::temp_dir().join(format!("algomln-parquet-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        let path = base.join("ABC.parquet");
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
                Arc::new(Int64Array::from(vec![10_i64])),
                Arc::new(Float64Array::from(vec![10.0])),
                Arc::new(Float64Array::from(vec![11.0])),
                Arc::new(Float64Array::from(vec![9.0])),
                Arc::new(Float64Array::from(vec![10.5])),
                Arc::new(Float64Array::from(vec![4.0])),
            ],
        )
        .unwrap();
        let mut writer = ArrowWriter::try_new(File::create(&path).unwrap(), schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        let source = FileSource::new(&base);
        let mut stream = source
            .open(&["ABC".into()], Timeframe::M1, DateRange::default())
            .unwrap();
        let (_, c) = stream.next().unwrap().unwrap();
        assert_eq!(c.timestamp, 10);
        assert_eq!(c.close, 10.5);
        assert!(stream.next().is_none());
        drop(stream);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn broker_waits_for_missing_symbol_and_partial_fills_cancel_cleanly() {
        let mut b = SimBroker::new(vec!["ABC".into()]);
        b.partial_fills_pct = Some(50.0);
        let req = OrderRequest {
            client_order_id: "partial".into(),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 5,
            reason: OrderReason::Rule("r".into()),
            ref_price: 10.0,
            decided_at: 1,
        };
        b.submit(&req).unwrap();
        b.on_candle_open(&BTreeMap::new());
        assert!(b.drain_events().is_empty());
        let mut c2 = candle(2, 10.0, 10.0);
        c2.volume = 2.0;
        b.on_candle_open(&BTreeMap::from([("ABC".into(), c2)]));
        let first = b.drain_events();
        assert!(matches!(first[0], OrderEvent::Acked { .. }));
        assert!(matches!(first[1], OrderEvent::PartialFill { qty: 1, .. }));
        let mut c3 = candle(3, 11.0, 11.0);
        c3.volume = 2.0;
        b.on_candle_open(&BTreeMap::from([("ABC".into(), c3)]));
        assert!(matches!(
            b.drain_events()[0],
            OrderEvent::PartialFill { qty: 1, .. }
        ));
        b.cancel("partial".into()).unwrap();
        assert!(matches!(b.drain_events()[0], OrderEvent::Cancelled { .. }));
        assert_eq!(
            b.cancel("partial".into()),
            Err(PortError::UnknownOrder("partial".into()))
        );
    }

    #[test]
    fn alphabetical_symbol_wins_when_cash_allows_one_entry() {
        let p = crate::compile("TRADE_IN B,A\nWHEN close > 0\nBUY 10\n").unwrap();
        let src = MemorySource(vec![
            ("B".into(), candle(1, 1.0, 1.0)),
            ("A".into(), candle(1, 1.0, 1.0)),
            ("A".into(), candle(2, 1.0, 1.0)),
            ("B".into(), candle(2, 1.0, 1.0)),
        ]);
        let mut cfg = config();
        cfg.initial_cash = 10.0;
        let r = run(&p, &src, cfg, None).unwrap();
        assert_eq!(r.fills.len(), 1);
        assert_eq!(r.fills[0].symbol, "A");
    }
}

/// Write deterministic, machine-readable run artifacts into an output folder.
pub fn write_results(dir: impl Into<PathBuf>, result: &RunResult) -> Result<(), std::io::Error> {
    let dir = dir.into();
    std::fs::create_dir_all(&dir)?;
    let mut f = csv::Writer::from_writer(File::create(dir.join("fills.csv"))?);
    csv_record(
        &mut f,
        [
            "ts",
            "symbol",
            "order_id",
            "side",
            "qty",
            "price",
            "price_paise",
            "fees",
            "fees_paise",
            "reason",
        ]
        .map(str::to_owned)
        .to_vec(),
    )?;
    for x in &result.fills {
        csv_record(
            &mut f,
            vec![
                x.ts.to_string(),
                x.symbol.clone(),
                x.order_id.clone(),
                format!("{:?}", x.side),
                x.qty.to_string(),
                format!("{:.10}", x.price),
                x.price_paise.to_string(),
                format!("{:.10}", x.fees),
                x.fees_paise.to_string(),
                format!("{:?}", x.reason),
            ],
        )?;
    }
    f.flush()?;
    let mut e = csv::Writer::from_writer(File::create(dir.join("equity.csv"))?);
    csv_record(
        &mut e,
        [
            "ts",
            "cash",
            "cash_paise",
            "positions_value",
            "positions_value_paise",
            "equity",
            "equity_paise",
        ]
        .map(str::to_owned)
        .to_vec(),
    )?;
    for x in &result.equity {
        csv_record(
            &mut e,
            vec![
                x.ts.to_string(),
                format!("{:.10}", x.cash),
                x.cash_paise.to_string(),
                format!("{:.10}", x.positions_value),
                x.positions_value_paise.to_string(),
                format!("{:.10}", x.equity),
                x.equity_paise.to_string(),
            ],
        )?;
    }
    e.flush()?;
    std::fs::write(
        dir.join("open_positions.json"),
        serde_json::to_vec_pretty(&result.open_positions).expect("serializable"),
    )?;
    std::fs::write(
        dir.join("events.json"),
        serde_json::to_vec_pretty(&result.events).expect("serializable"),
    )?;
    std::fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec_pretty(&result.manifest).expect("serializable"),
    )?;
    let start = result
        .manifest
        .pointer("/config/initial_cash")
        .and_then(serde_json::Value::as_f64)
        .or_else(|| result.equity.first().map(|x| x.equity))
        .unwrap_or(0.0);
    let start_paise = crate::trader::rupees_to_paise(start);
    let end_paise = result.equity.last().map_or(start_paise, |x| x.equity_paise);
    let mut peak_paise = start_paise;
    let mut max_dd = 0.0f64;
    for p in &result.equity {
        peak_paise = peak_paise.max(p.equity_paise);
        if peak_paise > 0 {
            max_dd = max_dd.max((peak_paise - p.equity_paise) as f64 / peak_paise as f64 * 100.0);
        }
    }
    let metrics = build_metrics(result, start_paise, end_paise, max_dd);
    std::fs::write(
        dir.join("metrics.json"),
        serde_json::to_vec_pretty(&metrics).expect("serializable"),
    )?;
    let mut t = csv::Writer::from_writer(File::create(dir.join("trades.csv"))?);
    csv_record(
        &mut t,
        [
            "symbol",
            "entry_ts",
            "exit_ts",
            "qty",
            "avg_entry",
            "avg_entry_paise",
            "avg_exit",
            "avg_exit_paise",
            "fees",
            "fees_paise",
            "net_pnl",
            "net_pnl_paise",
            "exit_reason",
        ]
        .map(str::to_owned)
        .to_vec(),
    )?;
    for tr in &result.trades {
        csv_record(
            &mut t,
            vec![
                tr.symbol.clone(),
                tr.entry_ts.to_string(),
                tr.exit_ts.to_string(),
                tr.qty.to_string(),
                format!("{:.10}", tr.avg_entry),
                tr.avg_entry_paise.to_string(),
                format!("{:.10}", tr.avg_exit),
                tr.avg_exit_paise.to_string(),
                format!("{:.10}", tr.fees),
                tr.fees_paise.to_string(),
                format!("{:.10}", tr.net_pnl),
                tr.net_pnl_paise.to_string(),
                format!("{:?}", tr.exit_reason),
            ],
        )?;
    }
    t.flush()?;
    Ok(())
}
fn csv_record(writer: &mut csv::Writer<File>, fields: Vec<String>) -> Result<(), std::io::Error> {
    writer.write_record(fields).map_err(csv_io_error)
}
fn csv_io_error(error: csv::Error) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}

fn build_metrics(
    result: &RunResult,
    start_paise: i64,
    end_paise: i64,
    portfolio_dd: f64,
) -> serde_json::Value {
    let exposure = |series: &[SymbolEquityPoint]| {
        let total = series
            .last()
            .zip(series.first())
            .map_or(0.0, |(b, a)| (b.ts - a.ts).max(0) as f64);
        let held: f64 = series
            .windows(2)
            .filter(|w| w[0].position_qty > 0)
            .map(|w| (w[1].ts - w[0].ts).max(0) as f64)
            .sum();
        if total == 0.0 {
            0.0
        } else {
            held / total * 100.0
        }
    };
    let stats = |trades: Vec<&Trade>| {
        let wins: Vec<_> = trades.iter().filter(|t| t.net_pnl_paise > 0).collect();
        let losses: Vec<_> = trades.iter().filter(|t| t.net_pnl_paise < 0).collect();
        let gross_win: i128 = wins.iter().map(|t| t.net_pnl_paise as i128).sum();
        let gross_loss: i128 = losses.iter().map(|t| -(t.net_pnl_paise as i128)).sum();
        serde_json::json!({"trades":trades.len(),"win_rate_pct":if trades.is_empty(){0.0}else{wins.len() as f64/trades.len() as f64*100.0},"profit_factor":if gross_loss==0{serde_json::Value::Null}else if gross_win==0{serde_json::json!(0.0)}else{serde_json::json!(gross_win as f64/gross_loss as f64)},"avg_win":if wins.is_empty(){0.0}else{crate::trader::paise_to_rupees(round_half_even_i128(gross_win, wins.len() as i128) as i64)},"avg_loss":if losses.is_empty(){0.0}else{crate::trader::paise_to_rupees(round_half_even_i128(gross_loss, losses.len() as i128) as i64)}})
    };
    let mut symbol_metrics = serde_json::Map::new();
    let n = result.symbol_equity.len().max(1) as i64;
    let allocation_base = start_paise / n;
    let allocation_remainder = start_paise.rem_euclid(n);
    for (index, (symbol, series)) in result.symbol_equity.iter().enumerate() {
        let allocation_paise = allocation_base + i64::from((index as i64) < allocation_remainder);
        let final_eq_paise = series.last().map_or(allocation_paise, |p| p.equity_paise);
        let mut peak_paise = allocation_paise;
        let mut dd = 0.0f64;
        for p in series {
            peak_paise = peak_paise.max(p.equity_paise);
            if peak_paise > 0 {
                dd = dd.max((peak_paise - p.equity_paise) as f64 / peak_paise as f64 * 100.0)
            }
        }
        let ts: Vec<_> = result
            .trades
            .iter()
            .filter(|t| &t.symbol == symbol)
            .collect();
        let mut item = stats(ts);
        if let Some(obj) = item.as_object_mut() {
            let net_pnl_paise = final_eq_paise - allocation_paise;
            obj.insert(
                "net_pnl".into(),
                serde_json::json!(crate::trader::paise_to_rupees(net_pnl_paise)),
            );
            obj.insert("net_pnl_paise".into(), serde_json::json!(net_pnl_paise));
            obj.insert(
                "return_pct".into(),
                serde_json::json!(if allocation_paise == 0 {
                    0.0
                } else {
                    net_pnl_paise as f64 / allocation_paise as f64 * 100.0
                }),
            );
            obj.insert("max_drawdown_pct".into(), serde_json::json!(dd));
            obj.insert("exposure_pct".into(), serde_json::json!(exposure(series)));
        }
        symbol_metrics.insert(symbol.clone(), item);
    }
    let mut blocked = BTreeMap::<String, u64>::new();
    let mut skipped = 0;
    let mut breach = 0;
    let mut rejects = 0;
    for ev in &result.events {
        match ev {
            TraderEvent::RuleSkipped { .. } => skipped += 1,
            TraderEvent::RiskBlocked { limit, .. } => {
                *blocked.entry((*limit).to_owned()).or_default() += 1
            }
            TraderEvent::DailyLossBreached => breach += 1,
            TraderEvent::OrderRejected { .. } => rejects += 1,
            _ => {}
        }
    }
    let flatten_orders = result
        .fills
        .iter()
        .filter(|f| f.reason == OrderReason::DailyLossFlatten)
        .map(|f| f.order_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    let portfolio_trades: Vec<_> = result.trades.iter().collect();
    let mut p = stats(portfolio_trades);
    let mut held_seconds = 0.0;
    let mut total_seconds = 0.0;
    if let Some(first) = result.equity.first() {
        if let Some(last) = result.equity.last() {
            total_seconds = (last.ts - first.ts).max(0) as f64;
            for w in result.equity.windows(2) {
                let any = result.symbol_equity.values().any(|s| {
                    s.iter()
                        .find(|x| x.ts == w[0].ts)
                        .is_some_and(|x| x.position_qty > 0)
                });
                if any {
                    held_seconds += (w[1].ts - w[0].ts).max(0) as f64;
                }
            }
        }
    }
    if let Some(obj) = p.as_object_mut() {
        let net_pnl_paise = end_paise - start_paise;
        obj.insert(
            "net_pnl".into(),
            serde_json::json!(crate::trader::paise_to_rupees(net_pnl_paise)),
        );
        obj.insert("net_pnl_paise".into(), serde_json::json!(net_pnl_paise));
        obj.insert("start_equity_paise".into(), serde_json::json!(start_paise));
        obj.insert("end_equity_paise".into(), serde_json::json!(end_paise));
        obj.insert(
            "return_pct".into(),
            serde_json::json!(if start_paise == 0 {
                0.0
            } else {
                net_pnl_paise as f64 / start_paise as f64 * 100.0
            }),
        );
        obj.insert("max_drawdown_pct".into(), serde_json::json!(portfolio_dd));
        obj.insert(
            "exposure_pct".into(),
            serde_json::json!(if total_seconds == 0.0 {
                0.0
            } else {
                held_seconds / total_seconds * 100.0
            }),
        );
        obj.insert(
            "open_positions".into(),
            serde_json::json!(result.open_positions.len()),
        );
        obj.insert("rules_skipped".into(), serde_json::json!(skipped));
        obj.insert("entries_blocked_by_gate".into(), serde_json::json!(blocked));
        obj.insert("daily_loss_breaches".into(), serde_json::json!(breach));
        obj.insert("flattens".into(), serde_json::json!(flatten_orders));
        obj.insert("rejects".into(), serde_json::json!(rejects));
        obj.insert(
            "missing_candles".into(),
            serde_json::to_value(&result.source_stats.missing_candles).unwrap_or_default(),
        );
        obj.insert(
            "skipped_bad".into(),
            serde_json::to_value(&result.source_stats.skipped_bad).unwrap_or_default(),
        );
    }
    serde_json::json!({"portfolio":p,"symbols":symbol_metrics})
}
