use dhan_api::feed::{FeedEvent, ReplaySource, TickSource};
use std::path::PathBuf;

#[test]
fn test_golden_capture_replay() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("capture.jsonl");

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
                assert!(t.price_paise >= 135930 && t.price_paise <= 136970);
                assert_eq!(t.security_id, 2885);
            }
            FeedEvent::PrevClose {
                security_id,
                price_paise,
                ..
            } => {
                prev_close_count += 1;
                assert_eq!(*security_id, 2885);
                assert!(*price_paise == 134480 || *price_paise == 136700);
            }
            _ => {}
        }
        events.push(ev);
    }

    assert_eq!(tick_count, 1243);
    assert_eq!(source.stale_ticks(), 3);
    assert_eq!(prev_close_count, 3);
    assert_eq!(source.dropped_duplicates, 48);
    assert_eq!(source.decode_errors, 0);
    assert_eq!(source.total_frames, 1297);

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
