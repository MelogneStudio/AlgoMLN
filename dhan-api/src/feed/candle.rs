use crate::feed::normalize::Tick;
use crate::feed::source::FeedEvent;
use std::collections::VecDeque;
use std::time::Duration;

pub const DEFAULT_GRACE_PERIOD_SECS: i64 = 1;

/// Represents a trading session window in IST seconds from midnight.
/// E.g. 09:15:00 is 33,300 seconds.
/// F&O session ends at 15:15:00 (54,900 seconds, 360 1m bars).
/// Equity non-F&O session ends at 15:30:00 (55,800 seconds, 375 1m bars).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionWindow {
    pub start_ist_sec: i64,
    pub end_ist_sec: i64,
}

impl SessionWindow {
    pub fn for_instrument(is_fno: bool) -> Self {
        if is_fno {
            // 09:15 to 15:15 (360 bars: 09:15 through 15:14:59)
            Self {
                start_ist_sec: 9 * 3600 + 15 * 60, // 33,300
                end_ist_sec: 15 * 3600 + 15 * 60,  // 54,900
            }
        } else {
            // 09:15 to 15:30 (375 bars: 09:15 through 15:29:59)
            Self {
                start_ist_sec: 9 * 3600 + 15 * 60, // 33,300
                end_ist_sec: 15 * 3600 + 30 * 60,  // 55,800
            }
        }
    }

    #[inline]
    pub fn is_in_session(&self, time_utc: i64) -> bool {
        let ist_sec = (time_utc + 19800).rem_euclid(86400);
        ist_sec >= self.start_ist_sec && ist_sec < self.end_ist_sec
    }

    #[inline]
    pub fn is_after_session(&self, time_utc: i64) -> bool {
        let ist_sec = (time_utc + 19800).rem_euclid(86400);
        ist_sec >= self.end_ist_sec
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClosedCandle {
    pub timestamp: i64, // start minute of candle in UTC epoch seconds
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    pub security_id: i32,
    pub filled: bool,
    pub gap: bool,
    pub partial_volume: bool,
}

#[derive(Clone, Debug)]
struct BuildingBar {
    start_ts: i64,
    security_id: i32,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    partial_volume: bool,
    gap: bool,
    tick_count: usize,
}

#[derive(Clone, Debug)]
pub struct CandleBuilderConfig {
    pub security_id: i32,
    pub is_fno: bool,
    pub grace_period: Duration,
    pub session_window: SessionWindow,
}

impl CandleBuilderConfig {
    pub fn new(security_id: i32, is_fno: bool) -> Self {
        Self {
            security_id,
            is_fno,
            grace_period: Duration::from_secs(DEFAULT_GRACE_PERIOD_SECS as u64),
            session_window: SessionWindow::for_instrument(is_fno),
        }
    }
}

pub struct CandleBuilder {
    config: CandleBuilderConfig,
    current_bar: Option<BuildingBar>,
    last_closed_close: Option<f64>,
    last_closed_minute: Option<i64>,
    current_clock_utc: i64,
    last_cum_volume: Option<i64>,
    baseline_volume_set: bool,
    disconnected: bool,
    pub off_session_ticks: Vec<Tick>,
    pub late_ticks: usize,
    closed_queue: VecDeque<ClosedCandle>,
}

impl CandleBuilder {
    pub fn new(config: CandleBuilderConfig) -> Self {
        Self {
            config,
            current_bar: None,
            last_closed_close: None,
            last_closed_minute: None,
            current_clock_utc: 0,
            last_cum_volume: None,
            baseline_volume_set: false,
            disconnected: false,
            off_session_ticks: Vec::new(),
            late_ticks: 0,
            closed_queue: VecDeque::new(),
        }
    }

    pub fn set_prev_close(&mut self, prev_close: f64) {
        if self.last_closed_close.is_none() {
            self.last_closed_close = Some(prev_close);
        }
    }

    /// Process a FeedEvent and update candles and clock.
    pub fn handle_event(&mut self, event: &FeedEvent, clock_utc: i64) {
        self.advance_clock(clock_utc);

        match event {
            FeedEvent::Tick(tick) => {
                if tick.security_id == self.config.security_id {
                    self.handle_tick(tick);
                }
            }
            FeedEvent::PrevClose {
                security_id,
                price_paise,
                ..
            } => {
                if *security_id == self.config.security_id {
                    self.set_prev_close(*price_paise as f64 / 100.0);
                }
            }
            FeedEvent::Connected => {
                self.disconnected = false;
                self.baseline_volume_set = false;
            }
            FeedEvent::Disconnected { .. } => {
                self.disconnected = true;
            }
            _ => {}
        }
    }

    /// Advance the clock and close bars whose minute + grace period has passed.
    /// Also emits flat filled candles for empty minutes within active sessions.
    pub fn advance_clock(&mut self, clock_utc: i64) {
        if clock_utc > self.current_clock_utc {
            self.current_clock_utc = clock_utc;
        }

        let grace_secs = self.config.grace_period.as_secs() as i64;

        // Check if the currently building bar should be closed
        if let Some(bar) = self.current_bar.take() {
            let bar_close_threshold = bar.start_ts + 60 + grace_secs;
            if self.current_clock_utc >= bar_close_threshold {
                let closed = ClosedCandle {
                    timestamp: bar.start_ts,
                    open: bar.open,
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                    volume: bar.volume,
                    security_id: bar.security_id,
                    filled: false,
                    gap: bar.gap,
                    partial_volume: bar.partial_volume,
                };
                self.last_closed_close = Some(closed.close);
                self.last_closed_minute = Some(bar.start_ts);
                self.closed_queue.push_back(closed);
            } else {
                self.current_bar = Some(bar);
            }
        }

        // Fill any empty minutes within session that have elapsed up to clock - 60 - grace
        if let (Some(last_close), Some(mut last_min)) = (self.last_closed_close, self.last_closed_minute) {
            let max_closeable_min = ((self.current_clock_utc - grace_secs) / 60) * 60 - 60;
            while last_min < max_closeable_min {
                let next_min = last_min + 60;
                // Only fill if inside trading session
                if self.config.session_window.is_in_session(next_min) {
                    let filled_candle = ClosedCandle {
                        timestamp: next_min,
                        open: last_close,
                        high: last_close,
                        low: last_close,
                        close: last_close,
                        volume: 0.0,
                        security_id: self.config.security_id,
                        filled: true,
                        gap: self.disconnected,
                        partial_volume: false,
                    };
                    self.closed_queue.push_back(filled_candle);
                    self.last_closed_minute = Some(next_min);
                    last_min = next_min;
                } else {
                    last_min = next_min;
                }
            }
        }
    }

    fn fill_gaps_up_to(&mut self, target_bar_start: i64) {
        if let (Some(last_close), Some(mut last_min)) = (self.last_closed_close, self.last_closed_minute) {
            while last_min + 60 < target_bar_start {
                let next_min = last_min + 60;
                if self.config.session_window.is_in_session(next_min) {
                    self.closed_queue.push_back(ClosedCandle {
                        timestamp: next_min,
                        open: last_close,
                        high: last_close,
                        low: last_close,
                        close: last_close,
                        volume: 0.0,
                        security_id: self.config.security_id,
                        filled: true,
                        gap: self.disconnected,
                        partial_volume: false,
                    });
                }
                last_min = next_min;
            }
            self.last_closed_minute = Some(last_min);
        }
    }

    /// Process a normalized tick.
    pub fn handle_tick(&mut self, tick: &Tick) {
        // 1. Check if tick is off-session (e.g. auction prints after 15:28 / pre-open)
        if !self.config.session_window.is_in_session(tick.time_utc) {
            self.off_session_ticks.push(*tick);
            return;
        }

        let bar_start_ts = (tick.time_utc / 60) * 60;

        // 2. Check if tick is late (its minute is already closed)
        if let Some(last_closed) = self.last_closed_minute {
            if bar_start_ts <= last_closed {
                self.late_ticks += 1;
                return;
            }
        }

        let price = tick.price_paise as f64 / 100.0;

        // 3. Track volume delta (quote mode)
        let (vol_delta, is_partial) = match tick.volume {
            Some(cum_vol) => {
                if !self.baseline_volume_set || self.last_cum_volume.is_none() {
                    self.last_cum_volume = Some(cum_vol);
                    self.baseline_volume_set = true;
                    (0.0, true)
                } else {
                    let prev = self.last_cum_volume.unwrap();
                    let delta = (cum_vol - prev).max(0) as f64;
                    self.last_cum_volume = Some(cum_vol);
                    (delta, false)
                }
            }
            None => (0.0, false),
        };

        // 4. Update current building bar
        match self.current_bar.as_mut() {
            Some(bar) if bar.start_ts == bar_start_ts => {
                bar.high = bar.high.max(price);
                bar.low = bar.low.min(price);
                bar.close = price;
                bar.volume += vol_delta;
                if is_partial {
                    bar.partial_volume = true;
                }
                bar.tick_count += 1;
            }
            Some(bar) => {
                // Minute changed while clock hasn't closed it yet:
                // Close old bar immediately or queue it
                let closed = ClosedCandle {
                    timestamp: bar.start_ts,
                    open: bar.open,
                    high: bar.high,
                    low: bar.low,
                    close: bar.close,
                    volume: bar.volume,
                    security_id: bar.security_id,
                    filled: false,
                    gap: bar.gap,
                    partial_volume: bar.partial_volume,
                };
                self.last_closed_close = Some(closed.close);
                self.last_closed_minute = Some(closed.timestamp);
                self.closed_queue.push_back(closed);

                self.fill_gaps_up_to(bar_start_ts);

                self.current_bar = Some(BuildingBar {
                    start_ts: bar_start_ts,
                    security_id: tick.security_id,
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                    volume: vol_delta,
                    partial_volume: is_partial,
                    gap: self.disconnected,
                    tick_count: 1,
                });
            }
            None => {
                self.fill_gaps_up_to(bar_start_ts);

                self.current_bar = Some(BuildingBar {
                    start_ts: bar_start_ts,
                    security_id: tick.security_id,
                    open: price,
                    high: price,
                    low: price,
                    close: price,
                    volume: vol_delta,
                    partial_volume: is_partial,
                    gap: self.disconnected,
                    tick_count: 1,
                });
            }
        }
    }

    /// Flush any remaining open bar at session close or feed termination.
    pub fn flush(&mut self) {
        if let Some(bar) = self.current_bar.take() {
            let closed = ClosedCandle {
                timestamp: bar.start_ts,
                open: bar.open,
                high: bar.high,
                low: bar.low,
                close: bar.close,
                volume: bar.volume,
                security_id: bar.security_id,
                filled: false,
                gap: bar.gap,
                partial_volume: bar.partial_volume,
            };
            self.last_closed_close = Some(closed.close);
            self.last_closed_minute = Some(closed.timestamp);
            self.closed_queue.push_back(closed);
        }
    }

    /// Returns next closed candle from the queue, if any.
    pub fn pop_candle(&mut self) -> Option<ClosedCandle> {
        self.closed_queue.pop_front()
    }
}
