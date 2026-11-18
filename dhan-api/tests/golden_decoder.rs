use dhan_api::feed::{decode, Packet};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

#[derive(Deserialize)]
struct FrameLine {
    t: f64,
    hex: String,
}

fn hex_to_bytes(hex: &str) -> Vec<u8> {
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
        .collect()
}

#[test]
fn test_golden_capture_decoding() {
    let fixture_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("capture.jsonl");

    let file = File::open(&fixture_path)
        .unwrap_or_else(|e| panic!("failed to open fixture {}: {e}", fixture_path.display()));
    let reader = BufReader::new(file);

    let mut packet_count = 0;
    let mut ticker_count = 0;
    let mut prev_close_count = 0;
    let mut prev_closes = HashSet::new();

    let mut session_ltts = Vec::new();
    let mut prices = Vec::new();

    for (line_idx, line_res) in reader.lines().enumerate() {
        let line = line_res.expect("read line");
        let frame: FrameLine = serde_json::from_str(&line).expect("parse frame json");
        assert!(frame.t > 0.0);

        let bytes = hex_to_bytes(&frame.hex);
        let (hdr, pkt) = decode(&bytes).unwrap_or_else(|e| {
            panic!("failed to decode line {line_idx} (len {}): {e}", bytes.len())
        });

        // Zero length mismatches
        assert_eq!(hdr.len as usize, bytes.len());
        packet_count += 1;

        match pkt {
            Packet::Ticker { ltp, ltt } => {
                ticker_count += 1;
                prices.push(ltp);
                // Lines 0 and 2 are subscriptions outside market hours / start of day
                // After subscription replay packets (index >= 2), ticks are within continuous session
                if line_idx >= 2 {
                    session_ltts.push(ltt);
                }
            }
            Packet::PrevClose { close, .. } => {
                prev_close_count += 1;
                let rounded = (close * 100.0).round() / 100.0;
                prev_closes.insert(format!("{:.1}", rounded));
            }
            _ => {}
        }
    }

    // 1. Assert exactly 1,297 packets decode
    assert_eq!(packet_count, 1297);
    assert_eq!(ticker_count, 1294);
    assert_eq!(prev_close_count, 3);

    // 2. Assert prev close values are 1344.8 and 1367.0
    assert!(prev_closes.contains("1344.8"), "missing 1344.8 in prev_closes: {prev_closes:?}");
    assert!(prev_closes.contains("1367.0"), "missing 1367.0 in prev_closes: {prev_closes:?}");

    // 3. Assert every price is inside the day's range [1359.30, 1369.70]
    let min_price = prices.iter().cloned().fold(f32::INFINITY, f32::min);
    let max_price = prices.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    assert!(min_price >= 1359.0 && min_price <= 1360.0, "unexpected min price: {min_price}");
    assert!(max_price >= 1369.0 && max_price <= 1370.0, "unexpected max price: {max_price}");
    for &p in &prices {
        assert!(p >= 1359.0 && p <= 1370.0, "price {p} outside day range");
    }

    // 4. Assert LTT never decreases within a session
    for i in 1..session_ltts.len() {
        assert!(
            session_ltts[i] >= session_ltts[i - 1],
            "LTT decreased from {} to {} at index {}",
            session_ltts[i - 1],
            session_ltts[i],
            i
        );
    }
}
