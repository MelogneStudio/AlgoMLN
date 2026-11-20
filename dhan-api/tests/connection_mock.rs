use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use dhan_api::feed::{
    batch_subscriptions, AuthError, FeedConfig, FeedEvent, InstrumentSubscription, LiveSource,
    RequestCode, SubscriptionError, TickSource,
};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::protocol::Message;

fn make_test_jwt(client_id: &str, exp: i64) -> String {
    let header = r#"{"alg":"HS256","typ":"JWT"}"#;
    let payload = format!(r#"{{"dhanClientId":"{}","exp":{}}}"#, client_id, exp);
    let h_b64 = URL_SAFE_NO_PAD.encode(header.as_bytes());
    let p_b64 = URL_SAFE_NO_PAD.encode(payload.as_bytes());
    format!("{h_b64}.{p_b64}.sig")
}

#[test]
fn test_expired_token_refusal() {
    let token = make_test_jwt("12345", 1000); // long expired
    let config = FeedConfig::new(token, vec![InstrumentSubscription::nse_eq("2885")]);
    let err = LiveSource::connect(config).err().expect("should fail");
    match err {
        dhan_api::feed::FeedError::Auth(AuthError::Expired { exp, .. }) => {
            assert_eq!(exp, 1000);
        }
        _ => panic!("expected AuthError::Expired, got {err:?}"),
    }
}

#[test]
fn test_subscribe_batching_rules() {
    let instruments: Vec<InstrumentSubscription> = (1..=250)
        .map(|i| InstrumentSubscription::nse_eq(i.to_string()))
        .collect();
    let batches = batch_subscriptions(RequestCode::Ticker, &instruments).unwrap();
    assert_eq!(batches.len(), 3);
    assert_eq!(batches[0].instrument_count, 100);
    assert_eq!(batches[1].instrument_count, 100);
    assert_eq!(batches[2].instrument_count, 50);

    let too_many: Vec<InstrumentSubscription> = (0..5001)
        .map(|i| InstrumentSubscription::nse_eq(i.to_string()))
        .collect();
    let err = batch_subscriptions(RequestCode::Ticker, &too_many).unwrap_err();
    assert_eq!(
        err,
        SubscriptionError::ExceedsConnectionLimit { count: 5001 }
    );
}

#[test]
fn test_mock_websocket_flow() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let (port_tx, port_rx) = std::sync::mpsc::channel();

    // Spawn mock WS server thread
    std::thread::spawn(move || {
        rt.block_on(async move {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            port_tx.send(port).unwrap();

            // Session 1: Client connects, sends subscribe, receives ticks, server disconnects
            let (tcp_stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(tcp_stream).await.unwrap();

            // 1. Read subscribe message
            let sub_msg = ws.next().await.unwrap().unwrap();
            assert!(sub_msg.to_text().unwrap().contains("2885"));

            // 2. Server sends ping (client handles it automatically)
            ws.send(Message::Ping(vec![1, 2, 3])).await.unwrap();
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

            // 3. Server sends ticker packet
            // Security 2885, segment 1, LTP 1300.8, fresh LTT
            let now_ltt = chrono::Utc::now().timestamp() as u32;
            let mut ticker_pkt: [u8; 16] = [
                0x02, 0x10, 0x00, 0x01, 0x45, 0x0B, 0x00, 0x00,
                0x9A, 0x99, 0xA2, 0x44, 0x00, 0x00, 0x00, 0x00,
            ];
            ticker_pkt[12..16].copy_from_slice(&now_ltt.to_le_bytes());
            ws.send(Message::Binary(ticker_pkt.to_vec())).await.unwrap();

            // 4. Close connection to trigger client reconnect
            ws.close(None).await.unwrap();

            // Session 2 (Reconnect): Client connects and resubscribes
            let (tcp_stream2, _) = listener.accept().await.unwrap();
            let mut ws2 = accept_async(tcp_stream2).await.unwrap();

            // Read resubscribe message
            let resub = ws2.next().await.unwrap().unwrap();
            assert!(resub.to_text().unwrap().contains("2885"));

            // Send code 50 disconnect packet with reason 805
            let mut disc_pkt = vec![0u8; 10];
            disc_pkt[0] = 50;
            disc_pkt[1..3].copy_from_slice(&10i16.to_le_bytes());
            disc_pkt[8..10].copy_from_slice(&805i16.to_le_bytes());
            ws2.send(Message::Binary(disc_pkt)).await.unwrap();
        });
    });

    let port = port_rx.recv().unwrap();

    let record_dir = std::env::temp_dir().join(format!("test_feed_rec_{}", std::process::id()));
    let record_path = record_dir.join("live_capture.jsonl");

    let future_exp = chrono::Utc::now().timestamp() + 3600;
    let token = make_test_jwt("mock_client", future_exp);

    let mut config = FeedConfig::new(token, vec![InstrumentSubscription::nse_eq("2885")]);
    config.base_url = format!("ws://127.0.0.1:{port}");
    config.initial_backoff = Duration::from_millis(50);
    config.max_backoff = Duration::from_millis(100);
    config = config.with_record_path(&record_path);

    let mut source = LiveSource::connect(config).unwrap();

    // 1. First event is Connected
    let ev1 = source.next().unwrap();
    assert_eq!(ev1, FeedEvent::Connected);

    // 2. Receive Ticker event
    let ev2 = source.next().unwrap();
    match ev2 {
        FeedEvent::Tick(t) => {
            assert_eq!(t.security_id, 2885);
            assert_eq!(t.price_paise, 130080);
        }
        _ => panic!("expected Tick, got {ev2:?}"),
    }

    // 3. Disconnect from first session drop
    let ev3 = source.next().unwrap();
    assert_eq!(ev3, FeedEvent::Disconnected { reason: None });

    // 4. Reconnect event
    let ev4 = source.next().unwrap();
    assert_eq!(ev4, FeedEvent::Connected);

    // 5. Code 805 Disconnected event
    let ev5 = source.next().unwrap();
    assert_eq!(ev5, FeedEvent::Disconnected { reason: Some(805) });

    // Verify recording file exists and has recorded the frames
    assert!(record_path.exists());
    let rec_content = std::fs::read_to_string(&record_path).unwrap();
    assert!(rec_content.contains(r#""hex": "02100001450b00009a99a244"#));

    source.stop();
    let _ = std::fs::remove_dir_all(record_dir);
}
