use crate::catalog::Catalog;
use crate::error::DataError;
use crate::session::{local_midnight, missing_date_ranges, ts_to_ist_date};
use crate::store::Store;
use crate::types::{Bar, Tf};
use algomln::{
    engine::Symbol,
    indicators::Candle,
    sim::{CandleSource, CandleStream, DateRange, Fingerprint, SourceError, Timeframe},
};
use arrow_array::{Array, Float64Array, Int64Array};
use chrono::{Datelike, NaiveDate};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use rusqlite::params;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
};

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
