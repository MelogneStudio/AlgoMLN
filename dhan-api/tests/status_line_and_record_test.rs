use dhan_api::feed::{ConnectionMetrics, LttConvention, RecordWriter};
use serde::Deserialize;
use std::fs::File;
use std::io::{BufRead, BufReader};

#[derive(Deserialize)]
struct FrameLine {
    t: f64,
    hex: String,
}

#[test]
fn test_status_line_contains_all_required_fields() {
    let mut metrics = ConnectionMetrics::new();
    metrics.record_packet(2, 1795145959.5);
    metrics.record_packet(2, 1795145960.0);
    metrics.record_packet(6, 1795145960.1);
    metrics.detected_convention = Some(LttConvention::Utc);
    metrics.last_ltt = Some(1795145959);
    metrics.last_recv_minus_ltt = Some(0.752);
    metrics.stale_ticks = 12;
    metrics.zero_ltt_ticks = 1;
    metrics.dropped_ticks = 5;
    metrics.reconnects = 2;

    let line = metrics.status_line();

    // Required fields:
    // 1. packets by code
    assert!(line.contains("code_2=2"), "missing code_2: {line}");
    assert!(line.contains("code_6=1"), "missing code_6: {line}");
    // 2. last LTT
    assert!(line.contains("last_ltt: 1795145959"), "missing last_ltt: {line}");
    // 3. receive-minus-LTT
    assert!(line.contains("recv-ltt: 0.752s"), "missing recv-ltt: {line}");
    // 4. detected convention
    assert!(line.contains("convention: Utc"), "missing convention: {line}");
    // 5. stale count
    assert!(line.contains("stale: 12"), "missing stale count: {line}");
    // 6. drops
    assert!(line.contains("drops: 5"), "missing drops count: {line}");
    // 7. reconnects
    assert!(line.contains("reconnects: 2"), "missing reconnects count: {line}");
}

#[test]
fn test_record_writes_every_frame_losslessly_including_short_and_unknown() {
    let dir = std::env::temp_dir().join(format!("test_lossless_record_{}", std::process::id()));
    let file_path = dir.join("capture.jsonl");

    let test_frames: Vec<(f64, Vec<u8>)> = vec![
        // Standard ticker frame (16 bytes, code 2)
        (
            1795145959.123456,
            vec![2, 16, 0, 0, 0x45, 0x0b, 0, 0, 0, 0, 0x80, 0x3f, 0, 0, 0, 0],
        ),
        // Unknown code frame (code 99, 16 bytes)
        (
            1795145960.000001,
            vec![99, 16, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        ),
        // Short frame (3 bytes, shorter than 8-byte header)
        (1795145961.500000, vec![0xde, 0xad, 0xbe]),
        // Empty frame (0 bytes)
        (1795145962.000000, vec![]),
        // Prev close frame (code 6, 16 bytes)
        (
            1795145963.999999,
            vec![6, 16, 0, 0, 0x45, 0x0b, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ),
    ];

    {
        let mut writer = RecordWriter::open(&file_path).expect("open writer");
        for (t, bytes) in &test_frames {
            writer.record_frame(*t, bytes).expect("record frame");
        }
    }

    // Read back and parse JSONL
    let file = File::open(&file_path).expect("open recorded file");
    let reader = BufReader::new(file);
    let mut read_lines = 0;

    for (idx, line_res) in reader.lines().enumerate() {
        let line = line_res.expect("read line");
        let parsed: FrameLine = serde_json::from_str(&line).expect("must parse JSONL");

        let expected = &test_frames[idx];
        assert!((parsed.t - expected.0).abs() < 1e-5);

        let mut expected_hex = String::new();
        for b in &expected.1 {
            use std::fmt::Write;
            let _ = write!(expected_hex, "{:02x}", b);
        }
        assert_eq!(parsed.hex, expected_hex);
        read_lines += 1;
    }

    assert_eq!(read_lines, test_frames.len(), "no frame dropped silently");

    let _ = std::fs::remove_dir_all(dir);
}
