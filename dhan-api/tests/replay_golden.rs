use dhan_api::feed::{FeedEvent, ReplaySource, TickSource};
use std::path::PathBuf;

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
fn test_golden_capture_replay() {
    let fixture_path = find_capture_path();

    let mut source = ReplaySource::from_file(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to open fixture {}: {e}", fixture_path.display()));

    // First event must be Connected
    assert_eq!(source.next(), Some(FeedEvent::Connected));

    let mut tick_count = 0;
    let mut prev_close_count = 0;
    let mut events = Vec::new();

    while let Some(ev) = source.next() {
        match &ev {
            FeedEvent::Tick(t) => {
                tick_count += 1;
                assert_eq!(t.security_id, 2885);
            }
            FeedEvent::PrevClose {
                security_id,
                price_paise: _,
                ..
            } => {
                prev_close_count += 1;
                assert_eq!(*security_id, 2885);
            }
            _ => {}
        }
        events.push(ev);
    }

    eprintln!(
        "tick_count: {}, prev_close_count: {}, dropped_duplicates: {}, stale_ticks: {}, zero_ltt: {}, decode_errors: {}, total_frames: {}",
        tick_count,
        prev_close_count,
        source.dropped_duplicates,
        source.stale_ticks(),
        source.zero_ltt_ticks(),
        source.decode_errors,
        source.total_frames
    );

    assert_eq!(source.total_frames, 7751);
    assert_eq!(source.stale_ticks(), 280);
    assert_eq!(source.zero_ltt_ticks(), 1);
    assert_eq!(prev_close_count, 4);
    assert_eq!(source.dropped_duplicates, 401);
    assert_eq!(source.decode_errors, 0);
    assert_eq!(tick_count, 7065);

    // Test replay determinism: run second time, verify identical sequence of events
    let mut source2 = ReplaySource::from_file(&fixture_path).unwrap();
    assert_eq!(source2.next(), Some(FeedEvent::Connected));

    let mut events2 = Vec::new();
    while let Some(ev) = source2.next() {
        events2.push(ev);
    }

    assert_eq!(events.len(), events2.len());
    for (i, (e1, e2)) in events.iter().zip(events2.iter()).enumerate() {
        assert_eq!(e1, e2, "event mismatch at index {i}");
    }
}
