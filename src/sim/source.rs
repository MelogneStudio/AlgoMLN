use arrow_array::{Array, Float64Array, Int64Array, RecordBatch};
use chrono::{DateTime, Datelike, FixedOffset, TimeZone};
use parquet::arrow::arrow_reader::{ParquetRecordBatchReader, ParquetRecordBatchReaderBuilder};
use serde::{Deserialize, Serialize};
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BinaryHeap},
    fs::File,
    io::Read,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use thiserror::Error;

use crate::{
    engine::Symbol,
    indicators::Candle,
};
use super::types::{DateRange, Fingerprint, GapPolicy, Timeframe};

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

pub(crate) fn limit_to_warmup_window(
    mut input: CandleStream,
    from: Option<i64>,
    bars: usize,
) -> CandleStream {
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

pub(crate) fn ist_day(ts: i64) -> i64 {
    ts + 19800 - (ts + 19800).rem_euclid(86400)
}

pub(crate) fn in_ist_session(ts: i64) -> bool {
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
