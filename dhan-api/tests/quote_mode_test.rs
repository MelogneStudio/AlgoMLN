use dhan_api::feed::{
    batch_subscriptions, FeedConfig, InstrumentSubscription, RequestCode,
    REQUEST_CODE_QUOTE, REQUEST_CODE_TICKER,
};

#[test]
fn test_named_request_codes_and_quote_mode_subscription() {
    // Assert single named constants
    assert_eq!(REQUEST_CODE_TICKER, 15);
    assert_eq!(REQUEST_CODE_QUOTE, 17); // UNVERIFIED quote mode request code

    assert_eq!(RequestCode::Ticker as u8, 15);
    assert_eq!(RequestCode::Quote as u8, 17);

    let instruments = vec![
        InstrumentSubscription::nse_eq("2885"),
        InstrumentSubscription::nse_eq("11536"),
    ];

    // Batch for ticker mode
    let ticker_batches = batch_subscriptions(RequestCode::Ticker, &instruments).unwrap();
    assert_eq!(ticker_batches.len(), 1);
    assert_eq!(ticker_batches[0].request_code, 15);

    // Batch for quote mode
    let quote_batches = batch_subscriptions(RequestCode::Quote, &instruments).unwrap();
    assert_eq!(quote_batches.len(), 1);
    assert_eq!(quote_batches[0].request_code, 17);

    let cfg = FeedConfig::new("dummy_token", instruments)
        .with_request_code(RequestCode::Quote);
    assert_eq!(cfg.request_code, RequestCode::Quote);
}

#[test]
fn test_raw_frame_inspector_hex_formatting() {
    let mut seen_code_counts = std::collections::HashMap::new();
    let mut printed = Vec::new();

    let simulate_frame = |bytes: &[u8], counts: &mut std::collections::HashMap<u8, usize>, out: &mut Vec<String>| {
        if !bytes.is_empty() {
            let code = bytes[0];
            let count = counts.entry(code).or_insert(0);
            if *count < 20 {
                *count += 1;
                let preview_len = bytes.len().min(16);
                let mut hex_preview = String::with_capacity(preview_len * 2);
                for b in &bytes[..preview_len] {
                    use std::fmt::Write;
                    let _ = write!(hex_preview, "{:02x}", b);
                }
                out.push(format!(
                    "code={}, len={}, hex={}, sample={}",
                    code,
                    bytes.len(),
                    hex_preview,
                    *count
                ));
            }
        }
    };

    // Simulate 25 frames of code 4 (quote response)
    let frame_code_4 = vec![4, 50, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
    for _ in 0..25 {
        simulate_frame(&frame_code_4, &mut seen_code_counts, &mut printed);
    }

    // Exactly first 20 frames are printed
    assert_eq!(printed.len(), 20);
    assert!(printed[0].contains("code=4, len=20, hex=043200000102030405060708090a0b0c, sample=1"));
    assert!(printed[19].contains("sample=20"));

    // A new code (e.g. 7) gets its own first 20 frames
    let frame_code_7 = vec![7, 8, 0, 0];
    for _ in 0..5 {
        simulate_frame(&frame_code_7, &mut seen_code_counts, &mut printed);
    }
    assert_eq!(printed.len(), 25);
    assert!(printed[20].contains("code=7, len=4, hex=07080000, sample=1"));
}
