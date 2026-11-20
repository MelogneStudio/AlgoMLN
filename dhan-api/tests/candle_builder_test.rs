use dhan_api::feed::{
    CandleBuilder, CandleBuilderConfig, ClosedCandle, FeedEvent, ReplaySource, Tick, TickSource,
};
use std::path::PathBuf;

#[derive(Debug, Default)]
pub struct AcceptanceDiff {
    pub total_bars: usize,
    pub open_mismatches: usize,
    pub high_mismatches: usize,
    pub low_mismatches: usize,
    pub close_mismatches: usize,
    pub volume_mismatches: usize,
    pub max_price_diff: f64,
}

impl AcceptanceDiff {
    pub fn report(&self) -> String {
        format!(
            "Acceptance diff ({} bars):\n  open mismatches: {}\n  high mismatches: {}\n  low mismatches: {}\n  close mismatches: {}\n  volume mismatches: {}\n  max price diff: {:.2}",
            self.total_bars,
            self.open_mismatches,
            self.high_mismatches,
            self.low_mismatches,
            self.close_mismatches,
            self.volume_mismatches,
            self.max_price_diff
        )
    }
}

pub fn compare_live_vs_api(
    live: &[ClosedCandle],
    api: &[(i64, f64, f64, f64, f64, f64)], // (ts, o, h, l, c, v)
) -> AcceptanceDiff {
    let mut diff = AcceptanceDiff {
        total_bars: live.len().min(api.len()),
        ..Default::default()
    };

    let api_map: std::collections::HashMap<i64, &(i64, f64, f64, f64, f64, f64)> =
        api.iter().map(|b| (b.0, b)).collect();

    for l in live {
        if let Some(a) = api_map.get(&l.timestamp) {
            let o_diff = (l.open - a.1).abs();
            let h_diff = (l.high - a.2).abs();
            let l_diff = (l.low - a.3).abs();
            let c_diff = (l.close - a.4).abs();

            if o_diff > 0.05 {
                diff.open_mismatches += 1;
            }
            if h_diff > 0.05 {
                diff.high_mismatches += 1;
            }
            if l_diff > 0.05 {
                diff.low_mismatches += 1;
            }
            if c_diff > 0.05 {
                diff.close_mismatches += 1;
            }
            if (l.volume - a.5).abs() > 1.0 {
                diff.volume_mismatches += 1;
            }

            let m = o_diff.max(h_diff).max(l_diff).max(c_diff);
            if m > diff.max_price_diff {
                diff.max_price_diff = m;
            }
        }
    }
    diff
}

#[test]
fn test_golden_capture_candle_builder() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("capture.jsonl");

    let run_replay = || -> (Vec<ClosedCandle>, usize, usize) {
        let mut source = ReplaySource::from_file(&fixture_path).expect("open capture");
        let config = CandleBuilderConfig::new(2885, true); // stock 2885 is F&O
        let mut builder = CandleBuilder::new(config);

        let mut candles = Vec::new();

        while let Some(ev) = source.next() {
            let clock_utc = source.current_time_utc().unwrap_or(0);
            builder.handle_event(&ev, clock_utc);

            while let Some(c) = builder.pop_candle() {
                candles.push(c);
            }
        }
        builder.flush();
        while let Some(c) = builder.pop_candle() {
            candles.push(c);
        }

        (candles, builder.off_session_ticks.len(), builder.late_ticks)
    };

    let (candles1, off_session1, late1) = run_replay();

    // 1. Bar counts: Exactly 20 continuous 1m bars from 14:55 to 15:14
    assert_eq!(candles1.len(), 20, "expected 20 1m candles, got {}", candles1.len());

    // 2. Dropped off-session ticks after 15:28: 2 ticks post-deduplication (auction replay + fresh auction tick)
    assert_eq!(off_session1, 2, "expected 2 deduplicated off-session auction ticks");

    // 3. Late ticks: 0
    assert_eq!(late1, 0, "expected 0 late ticks");

    // 4. Validate candle sanity
    for i in 0..candles1.len() {
        let c = &candles1[i];
        assert!(c.low <= c.open, "low > open in candle {i}");
        assert!(c.low <= c.close, "low > close in candle {i}");
        assert!(c.open <= c.high, "open > high in candle {i}");
        assert!(c.close <= c.high, "close > high in candle {i}");
        assert!(c.volume >= 0.0, "negative volume in candle {i}");
        assert!(!c.filled, "capture bars should not be flat filled");

        if i > 0 {
            assert_eq!(
                c.timestamp - candles1[i - 1].timestamp,
                60,
                "timestamps should increase by 60s"
            );
        }
    }

    // 5. Determinism: run twice, verify identical output
    let (candles2, off_session2, late2) = run_replay();
    assert_eq!(candles1, candles2, "replay must be strictly deterministic");
    assert_eq!(off_session1, off_session2);
    assert_eq!(late1, late2);

    // 6. Test acceptance diff helper with simulated identical API bars
    let mock_api_bars: Vec<(i64, f64, f64, f64, f64, f64)> = candles1
        .iter()
        .map(|c| (c.timestamp, c.open, c.high, c.low, c.close, c.volume))
        .collect();
    let diff = compare_live_vs_api(&candles1, &mock_api_bars);
    assert_eq!(diff.open_mismatches, 0);
    assert_eq!(diff.close_mismatches, 0);
    assert_eq!(diff.max_price_diff, 0.0);
}

#[test]
fn test_clock_based_closing_and_flat_fill() {
    let config = CandleBuilderConfig::new(2885, true);
    let mut builder = CandleBuilder::new(config);

    // Initial prev close at 1000.0
    builder.set_prev_close(1000.0);

    // Compute an exact 09:15:00 IST timestamp (seconds from midnight in IST = 33,300)
    let min_0915 = 1700000000 - ((1700000000 + 19800) % 86400) + 33300;
    assert_eq!((min_0915 + 19800) % 86400, 33300);

    // Tick at 09:15:10, price 1005.0 (100500 paise)
    let tick1 = Tick {
        security_id: 2885,
        price_paise: 100500,
        time_utc: min_0915 + 10,
        volume: None,
        recv_ns: 0,
    };
    builder.handle_event(&FeedEvent::Tick(tick1), min_0915 + 10);
    assert!(builder.pop_candle().is_none(), "bar should not close yet");

    // Clock reaches 09:15:59: still open
    builder.advance_clock(min_0915 + 59);
    assert!(builder.pop_candle().is_none(), "bar should not close at 59s");

    // Clock reaches 09:16:00: end of minute, but grace period is 1s, so not yet
    builder.advance_clock(min_0915 + 60);
    assert!(builder.pop_candle().is_none(), "grace period prevents early close");

    // Clock reaches 09:16:01 (minute end + 1s grace): BAR MUST CLOSE!
    builder.advance_clock(min_0915 + 61);
    let closed1 = builder.pop_candle().expect("bar should close on clock + grace");
    assert_eq!(closed1.timestamp, min_0915);
    assert_eq!(closed1.open, 1005.0);
    assert_eq!(closed1.close, 1005.0);
    assert!(!closed1.filled);

    // Now advance clock to 09:18:05 with NO ticks (empty minutes 09:16 and 09:17)
    builder.advance_clock(min_0915 + 185);

    // 09:16 empty minute should be emitted flat at 1005.0
    let filled_0916 = builder.pop_candle().expect("09:16 flat filled candle");
    assert_eq!(filled_0916.timestamp, min_0915 + 60);
    assert_eq!(filled_0916.open, 1005.0);
    assert_eq!(filled_0916.close, 1005.0);
    assert!(filled_0916.filled);

    // 09:17 empty minute should be emitted flat at 1005.0
    let filled_0917 = builder.pop_candle().expect("09:17 flat filled candle");
    assert_eq!(filled_0917.timestamp, min_0915 + 120);
    assert_eq!(filled_0917.open, 1005.0);
    assert_eq!(filled_0917.close, 1005.0);
    assert!(filled_0917.filled);
}

#[test]
fn test_tick_arrival_skipping_minutes_emits_carry_forward_flat_candles() {
    let config = CandleBuilderConfig::new(2885, true);
    let mut builder = CandleBuilder::new(config);

    let min_0915 = 1700000000 - ((1700000000 + 19800) % 86400) + 33300;

    // Tick at 09:15:10, price 1005.0
    let tick1 = Tick {
        security_id: 2885,
        price_paise: 100500,
        time_utc: min_0915 + 10,
        volume: None,
        recv_ns: 0,
    };
    builder.handle_event(&FeedEvent::Tick(tick1), min_0915 + 10);

    // No ticks during 09:16 or 09:17.
    // Next tick arrives directly at 09:18:15 with price 1010.0
    let tick2 = Tick {
        security_id: 2885,
        price_paise: 101000,
        time_utc: min_0915 + 195,
        volume: None,
        recv_ns: 0,
    };
    builder.handle_event(&FeedEvent::Tick(tick2), min_0915 + 195);

    // Closed candle 1: 09:15 (closed by minute change)
    let c1 = builder.pop_candle().expect("09:15 candle must be closed");
    assert_eq!(c1.timestamp, min_0915);
    assert_eq!(c1.close, 1005.0);
    assert!(!c1.filled);

    // Closed candle 2: 09:16 (flat fill gap)
    let c2 = builder.pop_candle().expect("09:16 flat filled candle must be emitted");
    assert_eq!(c2.timestamp, min_0915 + 60);
    assert_eq!(c2.close, 1005.0);
    assert!(c2.filled);

    // Closed candle 3: 09:17 (flat fill gap)
    let c3 = builder.pop_candle().expect("09:17 flat filled candle must be emitted");
    assert_eq!(c3.timestamp, min_0915 + 120);
    assert_eq!(c3.close, 1005.0);
    assert!(c3.filled);

    // 09:18 is currently building, not closed yet
    assert!(builder.pop_candle().is_none());
}
