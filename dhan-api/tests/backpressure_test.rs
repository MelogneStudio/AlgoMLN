use dhan_api::feed::{
    ConnectionMetrics, FeedConfig, FeedEvent, InstrumentSubscription, LiveSource, Tick,
};
use std::sync::mpsc::sync_channel;
use std::time::{Duration, Instant};

#[test]
fn test_channel_capacity_and_try_send_full_behavior() {
    let capacity = 5;
    let (tx, rx) = sync_channel::<FeedEvent>(capacity);
    let mut metrics = ConnectionMetrics::new();
    let mut last_drop_log = Instant::now() - Duration::from_secs(10);

    // Send exactly capacity events
    for i in 0..capacity {
        let tick = FeedEvent::Tick(Tick {
            security_id: 2885,
            price_paise: 100000 + i as i64,
            time_utc: 1700000000 + i as i64,
            volume: None,
            recv_ns: 0,
        });
        let sent = match tx.try_send(tick) {
            Ok(()) => true,
            Err(_) => false,
        };
        assert!(sent, "event {i} should succeed when below capacity");
    }

    assert_eq!(metrics.dropped_ticks, 0);

    // Now try to send 10 more events into full channel
    for i in 0..10 {
        let tick = FeedEvent::Tick(Tick {
            security_id: 2885,
            price_paise: 200000 + i as i64,
            time_utc: 1700000000 + (capacity + i) as i64,
            volume: None,
            recv_ns: 0,
        });

        match tx.try_send(tick) {
            Ok(()) => panic!("should not succeed on full channel"),
            Err(std::sync::mpsc::TrySendError::Full(_)) => {
                metrics.dropped_ticks += 1;
                // Rate-limited log simulation
                let now = Instant::now();
                if now.duration_since(last_drop_log) >= Duration::from_secs(5) {
                    last_drop_log = now;
                }
            }
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                panic!("channel should not be disconnected");
            }
        }
    }

    // Assert that exactly 10 drops were counted without panicking or blocking
    assert_eq!(metrics.dropped_ticks, 10);
    let status = metrics.status_line();
    assert!(status.contains("drops: 10"), "status line must expose drops: {status}");

    // Assert that original 5 events are intact and readable
    for i in 0..capacity {
        match rx.try_recv() {
            Ok(FeedEvent::Tick(t)) => {
                assert_eq!(t.price_paise, 100000 + i as i64);
            }
            other => panic!("expected Tick, got {:?}", other),
        }
    }

    // Now channel is empty
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_live_source_config_channel_capacity_and_metrics_exposure() {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    let header = r#"{"alg":"HS256","typ":"JWT"}"#;
    let payload = r#"{"dhanClientId":"123456","exp":2000000000}"#;
    let h_b64 = URL_SAFE_NO_PAD.encode(header.as_bytes());
    let p_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    let dummy_jwt = format!("{h_b64}.{p_b64}.sig");

    let config = FeedConfig::new(dummy_jwt, vec![InstrumentSubscription::nse_eq("2885")])
        .with_channel_capacity(42);

    assert_eq!(config.channel_capacity, 42);

    let live = LiveSource::connect(config).expect("connect LiveSource");
    assert_eq!(live.channel_capacity(), 42);
    assert_eq!(live.dropped_ticks(), 0);

    let m = live.metrics();
    assert_eq!(m.dropped_ticks, 0);
    assert!(m.status_line().contains("drops: 0"));

    live.stop();
}
