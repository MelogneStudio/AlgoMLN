use crate::source::CatalogSource;
use algomln::{
    engine::Symbol,
    indicators::Candle,
    sim::{CandleSource, CandleStream, DateRange, Fingerprint, SourceError, Timeframe},
};
use dhan_api::feed::{
    CandleBuilder, CandleBuilderConfig, FeedEvent, TickSource,
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

/// Connects live/replay Dhan feed ticks and optional historical catalog warm-up
/// to the AlgoMLN execution engine via the `CandleSource` interface.
pub struct LiveCandleSource {
    pub catalog_source: Option<CatalogSource>,
    pub feed_source: Arc<Mutex<Box<dyn TickSource + Send>>>,
    pub symbols_info: HashMap<Symbol, (i32, bool)>, // Symbol -> (security_id, is_fno)
}

impl LiveCandleSource {
    pub fn new(
        feed_source: Box<dyn TickSource + Send>,
        symbols_info: HashMap<Symbol, (i32, bool)>,
    ) -> Self {
        Self {
            catalog_source: None,
            feed_source: Arc::new(Mutex::new(feed_source)),
            symbols_info,
        }
    }

    pub fn with_catalog(mut self, catalog_source: CatalogSource) -> Self {
        self.catalog_source = Some(catalog_source);
        self
    }
}

impl CandleSource for LiveCandleSource {
    fn open(
        &self,
        symbols: &[Symbol],
        tf: Timeframe,
        range: DateRange,
    ) -> Result<CandleStream, SourceError> {
        let warmup_stream = if let Some(cat) = &self.catalog_source {
            Some(cat.open(symbols, tf, range)?)
        } else {
            None
        };

        let mut builders = HashMap::new();
        for sym in symbols {
            let (security_id, is_fno) = self.symbols_info.get(sym).copied().unwrap_or_else(|| {
                // Default fallback: parse security id if numeric, default is_fno = true
                let id: i32 = sym.parse().unwrap_or(0);
                (id, true)
            });
            let config = CandleBuilderConfig::new(security_id, is_fno);
            builders.insert(security_id, (sym.clone(), CandleBuilder::new(config)));
        }

        let stream = LiveCandleStream {
            warmup_stream,
            feed_source: Arc::clone(&self.feed_source),
            builders,
            queued_candles: VecDeque::new(),
            flushed: false,
        };

        Ok(Box::new(stream))
    }

    fn fingerprint(&self, symbols: &[Symbol], tf: Timeframe, range: DateRange) -> Fingerprint {
        let mut hasher = Sha256::new();
        hasher.update(b"live_candle_source_v1");
        for sym in symbols {
            hasher.update(sym.as_bytes());
        }
        hasher.update(tf.as_str().as_bytes());
        if let Some(from) = range.from {
            hasher.update(&from.to_le_bytes());
        }
        if let Some(to) = range.to {
            hasher.update(&to.to_le_bytes());
        }
        Fingerprint(format!("{:x}", hasher.finalize()))
    }
}

struct LiveCandleStream {
    warmup_stream: Option<CandleStream>,
    feed_source: Arc<Mutex<Box<dyn TickSource + Send>>>,
    builders: HashMap<i32, (Symbol, CandleBuilder)>,
    queued_candles: VecDeque<(Symbol, Candle)>,
    flushed: bool,
}

impl Iterator for LiveCandleStream {
    type Item = Result<(Symbol, Candle), SourceError>;

    fn next(&mut self) -> Option<Self::Item> {
        // 1. First, yield all historical warm-up candles
        if let Some(warmup) = &mut self.warmup_stream {
            match warmup.next() {
                Some(Ok((sym, candle))) => {
                    // Seed the corresponding builder's previous close
                    for (_, (b_sym, builder)) in self.builders.iter_mut() {
                        if b_sym == &sym {
                            builder.set_prev_close(candle.close);
                        }
                    }
                    return Some(Ok((sym, candle)));
                }
                Some(Err(e)) => return Some(Err(e)),
                None => {
                    // Warm-up exhausted, switch to live feed
                    self.warmup_stream = None;
                }
            }
        }

        // 2. Yield any queued closed candles
        if let Some(c) = self.queued_candles.pop_front() {
            return Some(Ok(c));
        }

        // 3. Process live feed events
        let mut feed = match self.feed_source.lock() {
            Ok(guard) => guard,
            Err(_) => return Some(Err(SourceError::Other("feed mutex poisoned".into()))),
        };

        loop {
            match feed.next() {
                Some(ev) => {
                    let clock_utc = feed.current_time_utc().unwrap_or(0);
                    match &ev {
                        FeedEvent::Tick(t) => {
                            if let Some((sym, builder)) = self.builders.get_mut(&t.security_id) {
                                builder.handle_event(&ev, clock_utc);
                                while let Some(c) = builder.pop_candle() {
                                    self.queued_candles.push_back((
                                        sym.clone(),
                                        Candle {
                                            timestamp: c.timestamp,
                                            open: c.open,
                                            high: c.high,
                                            low: c.low,
                                            close: c.close,
                                            volume: c.volume,
                                        },
                                    ));
                                }
                            }
                        }
                        FeedEvent::PrevClose { security_id, .. } => {
                            if let Some((_, builder)) = self.builders.get_mut(security_id) {
                                builder.handle_event(&ev, clock_utc);
                            }
                        }
                        _ => {
                            // Forward event to all builders (e.g. Connected/Disconnected)
                            for (_, (_, builder)) in self.builders.iter_mut() {
                                builder.handle_event(&ev, clock_utc);
                            }
                        }
                    }

                    if let Some(c) = self.queued_candles.pop_front() {
                        return Some(Ok(c));
                    }
                }
                None => {
                    // Feed ended: flush remaining builders once
                    if !self.flushed {
                        self.flushed = true;
                        for (_, (sym, builder)) in self.builders.iter_mut() {
                            builder.flush();
                            while let Some(c) = builder.pop_candle() {
                                self.queued_candles.push_back((
                                    sym.clone(),
                                    Candle {
                                        timestamp: c.timestamp,
                                        open: c.open,
                                        high: c.high,
                                        low: c.low,
                                        close: c.close,
                                        volume: c.volume,
                                    },
                                ));
                            }
                        }
                    }

                    return self.queued_candles.pop_front().map(Ok);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dhan_api::feed::ReplaySource;
    use std::path::PathBuf;

    #[test]
    fn test_live_candle_source_replay_stream() {
        let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("dhan-api")
            .join("tests")
            .join("fixtures")
            .join("capture.jsonl");

        let replay = ReplaySource::from_file(&fixture_path).expect("open capture");
        let mut symbols_info = HashMap::new();
        symbols_info.insert("RELIANCE".to_string(), (2885, true));

        let source = LiveCandleSource::new(Box::new(replay), symbols_info);

        let stream = source
            .open(
                &["RELIANCE".to_string()],
                Timeframe::M1,
                DateRange::default(),
            )
            .expect("open stream");

        let candles: Vec<(Symbol, Candle)> = stream.map(|r| r.expect("candle")).collect();

        // 20 bars from golden capture
        assert_eq!(candles.len(), 20);
        assert_eq!(candles[0].0, "RELIANCE");
        assert!(candles[0].1.open >= 1359.0);
        assert!(candles[0].1.close <= 1370.0);

        for i in 1..candles.len() {
            assert_eq!(candles[i].1.timestamp - candles[i - 1].1.timestamp, 60);
        }
    }
}
