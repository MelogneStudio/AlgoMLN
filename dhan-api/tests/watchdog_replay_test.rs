use dhan_api::feed::{
    FeedWatchdog, ManualFeedClock, SessionWindow, WatchdogAction,
};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

#[derive(Deserialize)]
struct FrameLine {
    t: f64,
    #[allow(dead_code)]
    hex: String,
}

fn find_capture_path() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let candidates = [
        manifest.join("..").join("sample-data").join("capture.jsonl"),
        manifest.join("sample-data").join("capture.jsonl"),
        manifest.join("tests").join("fixtures").join("capture.jsonl"),
    ];
    for p in &candidates {
        if p.exists() {
            return p.clone();
        }
    }
    candidates[0].clone()
}

#[test]
fn test_watchdog_replay_capture_never_fires_during_normal_silences() {
    let fixture_path = find_capture_path();
    let file = File::open(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to open fixture {}: {e}", fixture_path.display()));
    let reader = BufReader::new(file);

    let clock = Arc::new(ManualFeedClock::new(0));
    // F&O instrument: 09:15 to 15:15 IST
    let mut watchdog = FeedWatchdog::new(
        SessionWindow::for_instrument(true),
        Duration::from_secs(40),
        clock.clone(),
    );

    let mut prev_t_sec = 0i64;
    let mut total_packets = 0;

    let mut checked_0847_silence = false;
    let mut checked_0909_silence = false;
    let mut checked_nov18_1515_silence = false;

    for (line_idx, line_res) in reader.lines().enumerate() {
        let line = line_res.expect("read line");
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let frame: FrameLine = serde_json::from_str(trimmed).expect("parse frame json");
        let t_sec = frame.t as i64;
        total_packets += 1;

        let ist_day = (t_sec + 19800).div_euclid(86400);
        let prev_day = if prev_t_sec > 0 {
            (prev_t_sec + 19800).div_euclid(86400)
        } else {
            ist_day
        };

        // If within same day session and time advances
        if prev_t_sec > 0 && t_sec > prev_t_sec && ist_day == prev_day {
            // Check every elapsed second across any gaps within session
            for s in (prev_t_sec + 1)..=t_sec {
                clock.set(s);
                let status = watchdog.check(s);

                // Watchdog must NEVER fire a timeout across the recorded capture sessions
                assert_ne!(
                    status,
                    WatchdogAction::TriggeredTimeout {
                        elapsed_secs: 40,
                        timeout_secs: 40
                    },
                    "watchdog unexpectedly fired at second {s} (line {line_idx})"
                );

                let ist_sec = (s + 19800).rem_euclid(86400);

                // Silence 1: 08:47 to 09:00 IST (31,620s to 32,400s)
                if ist_sec >= 31620 && ist_sec < 32400 {
                    assert_eq!(status, WatchdogAction::Disarmed, "must be disarmed 08:47-09:00");
                    checked_0847_silence = true;
                }

                // Silence 2: 09:09:19 to 09:15:00 IST (32,959s to 33,300s)
                if ist_sec >= 32959 && ist_sec < 33300 {
                    assert_eq!(status, WatchdogAction::Disarmed, "must be disarmed 09:09-09:15");
                    checked_0909_silence = true;
                }

                // Silence 3: Nov 18 15:15:00 to 15:28:46 IST (54,900s to 55,726s)
                if ist_sec >= 54900 && ist_sec < 55726 {
                    assert_eq!(status, WatchdogAction::Disarmed, "must be disarmed post 15:15 for F&O");
                    checked_nov18_1515_silence = true;
                }
            }
        }

        prev_t_sec = t_sec;
        clock.set(t_sec);
        watchdog.on_activity(t_sec);
    }

    assert_eq!(total_packets, 7751);
    assert!(checked_0847_silence, "verified 08:47 silence");
    assert!(checked_0909_silence, "verified 09:09 silence");
    assert!(checked_nov18_1515_silence, "verified Nov 18 15:15 silence");
}

#[test]
fn test_watchdog_fires_on_in_session_stall() {
    let clock = Arc::new(ManualFeedClock::new(1795146300)); // 2026-11-20 09:15:00 IST
    let mut watchdog = FeedWatchdog::new(
        SessionWindow::for_instrument(true),
        Duration::from_secs(40),
        clock.clone(),
    );

    // Activity at 09:30:00 IST (1795147200 UTC)
    let t_0930 = 1795147200;
    clock.set(t_0930);
    watchdog.on_activity(t_0930);
    assert_eq!(watchdog.check_now(), WatchdogAction::Ok);

    // Advance 39 seconds: still Ok
    clock.set(t_0930 + 39);
    assert_eq!(watchdog.check_now(), WatchdogAction::Ok);

    // Advance to 40 seconds: Watchdog FIRES!
    clock.set(t_0930 + 40);
    assert_eq!(
        watchdog.check_now(),
        WatchdogAction::TriggeredTimeout {
            elapsed_secs: 40,
            timeout_secs: 40,
        }
    );
}

#[test]
fn test_watchdog_cash_vs_fno_window_difference() {
    // 2026-11-18: midnight IST is 1794940200 UTC.
    // 15:15:00 IST (54,900s) is 1794940200 + 54900 = 1794995100 UTC
    let t_1515 = 1794995100;
    let clock_fno = Arc::new(ManualFeedClock::new(t_1515));
    let mut wd_fno = FeedWatchdog::new(
        SessionWindow::for_instrument(true), // F&O ends 15:15 (54,900s)
        Duration::from_secs(40),
        clock_fno.clone(),
    );

    let clock_cash = Arc::new(ManualFeedClock::new(t_1515));
    let mut wd_cash = FeedWatchdog::new(
        SessionWindow::for_instrument(false), // Cash ends 15:30 (55,800s)
        Duration::from_secs(40),
        clock_cash.clone(),
    );

    // At 15:15:00, F&O is already at end of session -> Disarmed!
    assert_eq!(wd_fno.check_now(), WatchdogAction::Disarmed);

    // But Cash is still inside session (runs until 15:30) -> Armed!
    assert_eq!(wd_cash.check_now(), WatchdogAction::Ok);

    // After 40s of silence at 15:15:40:
    clock_fno.advance(40);
    clock_cash.advance(40);

    // F&O remains Disarmed (does not fire)
    assert_eq!(wd_fno.check_now(), WatchdogAction::Disarmed);

    // Cash fires timeout because it is inside active trading hours!
    assert_eq!(
        wd_cash.check_now(),
        WatchdogAction::TriggeredTimeout {
            elapsed_secs: 40,
            timeout_secs: 40,
        }
    );
}
