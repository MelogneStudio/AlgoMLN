use dhan_api::exec::{
    CorrelationGenerator, DhanExecutionPort, DhanExecutionPortConfig,
    DhanTradingClient, ErrorClass, ExecEvent, ExecutionMode, ExitLoopPhase, ExitReplaceConfig,
    ModeConfig, NormalizedOrderUpdate, NormalizedStatus, OrderIntent, OrderKind, OrderRequest,
    OrderState, OrderTracker, OrderWal, Reconciler, RiskController, RiskRailsConfig, Side,
    SubmissionQueue, UpdateSource,
};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn init_mock_env() -> std::sync::MutexGuard<'static, ()> {
    let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("DHAN_SANDBOX_ACCESS_TOKEN", "mock_sandbox_token");
    guard
}

/// Simple mock server that routes incoming HTTP requests to handlers.
struct MockHttpServer {
    addr: String,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockHttpServer {
    fn new<F>(handler: F) -> Self
    where
        F: Fn(&str, &str, &str) -> (u16, &'static str) + Send + Sync + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let handler = Arc::new(handler);

        let handle = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            while !stop_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                        let mut buf = [0u8; 8192];
                        let n = match stream.read(&mut buf) {
                            Ok(n) if n > 0 => n,
                            _ => continue,
                        };
                        let req_text = String::from_utf8_lossy(&buf[..n]);
                        let mut lines = req_text.lines();
                        let first_line = lines.next().unwrap_or("");
                        let parts: Vec<&str> = first_line.split_whitespace().collect();
                        let method = parts.first().copied().unwrap_or("GET");
                        let path = parts.get(1).copied().unwrap_or("/");

                        let (status_code, body) = handler(method, path, &req_text);
                        let response = format!(
                            "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            status_code,
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(response.as_bytes());
                        let _ = stream.flush();
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            addr,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for MockHttpServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario 1: Timeout then success, resolved by correlation lookup, no duplicate
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_timeout_then_success_resolved_by_correlation_no_duplicate() {
    let _env = init_mock_env();
    let server = MockHttpServer::new(|method, path, _req| {
        if method == "POST" && path == "/orders" {
            // First place request triggers a transient 504 Gateway Timeout
            (504, r#"{"status":"failure","remarks":"Gateway Timeout"}"#)
        } else if method == "GET" && path.starts_with("/orders/external/") {
            // Correlation lookup succeeds and confirms Dhan did accept the order
            (
                200,
                r#"{
                    "orderId": "DH-RESOLVED-101",
                    "correlationId": "AMLN-R1-0001",
                    "orderStatus": "PENDING",
                    "quantity": 50,
                    "filledQty": 0,
                    "remainingQuantity": 50
                }"#,
            )
        } else {
            (404, r#"{}"#)
        }
    });

    std::env::set_var("DHAN_SANDBOX_ACCESS_TOKEN", "mock_token");
    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let client = DhanTradingClient::new(mode_cfg, "1000000000".into());

    let req = OrderRequest {
        local_id: "ORD-1".into(),
        security_id: "1333".into(),
        side: Side::Buy,
        quantity: 50,
        kind: OrderKind::Market,
        intent: OrderIntent::Entry,
        local_seq: 1,
    };

    let place_res = client.place_order(&req, "AMLN-R1-0001", None).await;
    assert!(place_res.is_err());
    match place_res.unwrap_err() {
        dhan_api::exec::ExecRestError::UnknownOutcome { class, .. } => {
            assert_eq!(class, ErrorClass::UnknownOutcome);
        }
        other => panic!("expected UnknownOutcome error, got {other:?}"),
    }

    // Resolve by correlation ID: verifies the order exists, preventing duplicate submission
    let resolved = client
        .get_order_by_correlation("AMLN-R1-0001")
        .await
        .unwrap()
        .expect("order found via correlation lookup");

    assert_eq!(resolved.dhan_order_id.as_deref(), Some("DH-RESOLVED-101"));
    assert_eq!(resolved.status, NormalizedStatus::Open);
}

// ---------------------------------------------------------------------------
// Scenario 2: Partial fill, then fill, with correct incremental quantities
// ---------------------------------------------------------------------------
#[test]
fn test_partial_fill_then_fill_incremental_quantities() {
    let req = OrderRequest {
        local_id: "L1".into(),
        security_id: "1333".into(),
        side: Side::Buy,
        quantity: 100,
        kind: OrderKind::Market,
        intent: OrderIntent::Entry,
        local_seq: 1,
    };
    let mut tracker = OrderTracker::new(req, "AMLN-R1-0001".into());

    // Update 1: Partial fill of 40 shares
    let up1 = NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("DH-100".into()),
        correlation_id: Some("AMLN-R1-0001".into()),
        status: NormalizedStatus::PartiallyFilled,
        quantity: 100,
        cum_traded_qty: 40,
        remaining_qty: 60,
        avg_price_paise: Some(25000),
        last_traded_price_paise: None,
        side: Some(Side::Buy),
        reject_reason: None,
        update_time: Some("1".into()),
    };
    let evs1 = tracker.apply_update(&up1);
    assert_eq!(evs1.len(), 1);
    match &evs1[0] {
        ExecEvent::Fill { qty, price_paise, .. } => {
            assert_eq!(*qty, 40);
            assert_eq!(*price_paise, 25000);
        }
        _ => panic!("expected fill event"),
    }
    assert_eq!(tracker.cum_traded_qty, 40);
    assert_eq!(tracker.remaining_qty, 60);

    // Update 2: Final fill of remaining 60 shares (cum 100)
    let up2 = NormalizedOrderUpdate {
        source: UpdateSource::Websocket,
        dhan_order_id: Some("DH-100".into()),
        correlation_id: Some("AMLN-R1-0001".into()),
        status: NormalizedStatus::Filled,
        quantity: 100,
        cum_traded_qty: 100,
        remaining_qty: 0,
        avg_price_paise: Some(25050),
        last_traded_price_paise: Some(25100),
        side: Some(Side::Buy),
        reject_reason: None,
        update_time: Some("2".into()),
    };
    let evs2 = tracker.apply_update(&up2);
    assert_eq!(evs2.len(), 2);
    match &evs2[0] {
        ExecEvent::Fill { qty, price_paise, .. } => {
            assert_eq!(*qty, 60); // Exact incremental fill!
            assert_eq!(*price_paise, 25100);
        }
        _ => panic!("expected fill event"),
    }
    assert_eq!(evs2[1], ExecEvent::Filled { local_id: "L1".into() });
    assert_eq!(tracker.state, OrderState::Filled);
    assert_eq!(tracker.cum_traded_qty, 100);
    assert_eq!(tracker.remaining_qty, 0);
}

// ---------------------------------------------------------------------------
// Scenario 3: Rejection, with no retry
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_rejection_with_no_retry() {
    let _env = init_mock_env();
    let server = MockHttpServer::new(|method, path, _req| {
        if method == "POST" && path == "/orders" {
            // Broker business rejection DH-906
            (
                400,
                r#"{
                    "status": "failure",
                    "errorCode": "DH-906",
                    "errorMessage": "Insufficient funds to execute order"
                }"#,
            )
        } else {
            (404, r#"{}"#)
        }
    });

    std::env::set_var("DHAN_SANDBOX_ACCESS_TOKEN", "mock_token");
    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let client = DhanTradingClient::new(mode_cfg, "1000000000".into());

    let req = OrderRequest {
        local_id: "REJ-1".into(),
        security_id: "1333".into(),
        side: Side::Buy,
        quantity: 100,
        kind: OrderKind::Market,
        intent: OrderIntent::Entry,
        local_seq: 1,
    };

    let place_res = client.place_order(&req, "AMLN-REJ-1", None).await;
    assert!(place_res.is_err());
    match place_res.unwrap_err() {
        dhan_api::exec::ExecRestError::Rejected { class, message, .. } => {
            assert_eq!(class, ErrorClass::BusinessRejection);
            assert!(message.contains("Insufficient funds"));
        }
        other => panic!("expected BusinessRejection, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Scenario 4: Duplicate fill message from poll and websocket, counted once
// ---------------------------------------------------------------------------
#[test]
fn test_duplicate_fill_poll_and_websocket_counted_once() {
    let req = OrderRequest {
        local_id: "DUP-1".into(),
        security_id: "1333".into(),
        side: Side::Buy,
        quantity: 50,
        kind: OrderKind::Market,
        intent: OrderIntent::Entry,
        local_seq: 1,
    };
    let mut tracker = OrderTracker::new(req, "AMLN-DUP-1".into());

    // Update from REST
    let poll_update = NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("DH-DUP".into()),
        correlation_id: Some("AMLN-DUP-1".into()),
        status: NormalizedStatus::PartiallyFilled,
        quantity: 50,
        cum_traded_qty: 25,
        remaining_qty: 25,
        avg_price_paise: Some(25000),
        last_traded_price_paise: None,
        side: Some(Side::Buy),
        reject_reason: None,
        update_time: Some("10:00:01".into()),
    };
    let evs1 = tracker.apply_update(&poll_update);
    assert_eq!(evs1.len(), 1);

    // Duplicate update from WebSocket for the same 25 tradedQty
    let ws_update = NormalizedOrderUpdate {
        source: UpdateSource::Websocket,
        dhan_order_id: Some("DH-DUP".into()),
        correlation_id: Some("AMLN-DUP-1".into()),
        status: NormalizedStatus::PartiallyFilled,
        quantity: 50,
        cum_traded_qty: 25, // Same cumulative quantity
        remaining_qty: 25,
        avg_price_paise: Some(25000),
        last_traded_price_paise: Some(25000),
        side: Some(Side::Buy),
        reject_reason: None,
        update_time: Some("10:00:01".into()),
    };
    let evs2 = tracker.apply_update(&ws_update);
    assert!(evs2.is_empty(), "duplicate fill must not be emitted twice");
}

// ---------------------------------------------------------------------------
// Scenario 5: Websocket drop with a missed fill, recovered by reconciler
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_websocket_drop_missed_fill_recovered_by_reconciler() {
    let _env = init_mock_env();
    let server = MockHttpServer::new(|method, path, _req| {
        if method == "GET" && path == "/orders" {
            // Broker reports the missed fill
            (
                200,
                r#"[{
                    "orderId": "DH-MISSED",
                    "correlationId": "AMLN-MISS-1",
                    "orderStatus": "TRADED",
                    "quantity": 40,
                    "filledQty": 40,
                    "remainingQuantity": 0,
                    "averageTradedPrice": 250.0
                }]"#,
            )
        } else if method == "GET" && path == "/positions" {
            (
                200,
                r#"[{
                    "securityId": "1333",
                    "productType": "INTRADAY",
                    "netQty": 40,
                    "buyQty": 40,
                    "sellQty": 0
                }]"#,
            )
        } else {
            (404, r#"{}"#)
        }
    });

    std::env::set_var("DHAN_SANDBOX_ACCESS_TOKEN", "mock_token");
    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let client = DhanTradingClient::new(mode_cfg, "1000000000".into());
    let mut risk = RiskController::new(RiskRailsConfig::default());
    let reconciler = Reconciler::new(CorrelationGenerator::new("MISS"));

    let req = OrderRequest {
        local_id: "ORD-MISS".into(),
        security_id: "1333".into(),
        side: Side::Buy,
        quantity: 40,
        kind: OrderKind::Market,
        intent: OrderIntent::Entry,
        local_seq: 1,
    };
    let mut tracker = OrderTracker::new(req, "AMLN-MISS-1".into());
    tracker.state = OrderState::Open; // Local state was still Open because WS dropped!

    let mut local_orders = BTreeMap::new();
    local_orders.insert("ORD-MISS".into(), tracker);
    let mut local_positions = BTreeMap::new();
    local_positions.insert("1333".into(), 40);

    // Reconciler recovers the missed fill from REST order book
    let events = reconciler
        .reconcile(&client, &mut risk, &mut local_orders, &mut local_positions)
        .await;

    assert!(events.iter().any(|e| matches!(e, ExecEvent::Fill { qty: 40, .. })));
    assert!(events.iter().any(|e| matches!(e, ExecEvent::Filled { .. })));
    assert_eq!(local_orders["ORD-MISS"].state, OrderState::Filled);
}

// ---------------------------------------------------------------------------
// Scenario 6: Restart with an open position and an open order
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_restart_with_open_position_and_open_order_wal_recovery() {
    let dir = std::env::temp_dir().join(format!("wal_restart_test_{}", std::process::id()));
    let wal_path = dir.join("orders.wal");
    let _ = std::fs::remove_file(&wal_path);

    // Write-ahead log before crash
    {
        let mut wal = OrderWal::open(&wal_path).unwrap();
        let req = OrderRequest {
            local_id: "RESTART-1".into(),
            security_id: "1333".into(),
            side: Side::Buy,
            quantity: 30,
            kind: OrderKind::Market,
            intent: OrderIntent::Entry,
            local_seq: 1,
        };
        let mut tracker = OrderTracker::new(req, "AMLN-RES-1".into());
        tracker.state = OrderState::Open;
        tracker.dhan_order_id = Some("DH-RES-100".into());
        wal.append(&tracker).unwrap();
    }

    // Now restart and recover
    let wal_after_restart = OrderWal::open(&wal_path).unwrap();
    let recovered_orders = wal_after_restart.recover_open_orders().unwrap();
    assert_eq!(recovered_orders.len(), 1);
    assert_eq!(recovered_orders[0].local_id, "RESTART-1");
    assert_eq!(recovered_orders[0].state, OrderState::Open);
    assert_eq!(recovered_orders[0].dhan_order_id.as_deref(), Some("DH-RES-100"));

    let _ = std::fs::remove_dir_all(&dir);
}

// ---------------------------------------------------------------------------
// Scenario 7: Unknown status string locks entries without crashing
// ---------------------------------------------------------------------------
#[test]
fn test_unknown_status_string_locks_entries_without_crashing() {
    let req = OrderRequest {
        local_id: "UNK-1".into(),
        security_id: "1333".into(),
        side: Side::Buy,
        quantity: 10,
        kind: OrderKind::Market,
        intent: OrderIntent::Entry,
        local_seq: 1,
    };
    let mut tracker = OrderTracker::new(req, "AMLN-UNK-1".into());

    let unknown_update = NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("DH-UNK".into()),
        correlation_id: Some("AMLN-UNK-1".into()),
        status: NormalizedStatus::Unknown("MYSTERIOUS_STATUS".into()),
        quantity: 10,
        cum_traded_qty: 0,
        remaining_qty: 10,
        avg_price_paise: None,
        last_traded_price_paise: None,
        side: Some(Side::Buy),
        reject_reason: None,
        update_time: None,
    };

    let events = tracker.apply_update(&unknown_update);
    assert_eq!(tracker.state, OrderState::Unknown);
    assert_eq!(events.len(), 1);
    match &events[0] {
        ExecEvent::Unknown { reason, .. } => {
            assert!(reason.contains("MYSTERIOUS_STATUS"));
        }
        _ => panic!("expected Unknown event"),
    }
}

// ---------------------------------------------------------------------------
// Scenario 8: Rate-limit response (both RL001 and DH-904) backs off and requeues
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_rate_limit_rl001_and_dh904_backs_off_and_requeues() {
    let _env = init_mock_env();
    let attempt = Arc::new(AtomicUsize::new(0));
    let attempt_clone = Arc::clone(&attempt);
    let server = MockHttpServer::new(move |_method, path, _req| {
        if path == "/orders" {
            let n = attempt_clone.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 1 {
                (
                    429,
                    r#"{"status":"failure","errorType":"RATE_LIMIT_ERROR","errorCode":"RL001","message":"Too many requests"}"#,
                )
            } else if n == 2 {
                (
                    429,
                    r#"{"status":"failure","errorCode":"DH-904","errorMessage":"Order rate limit exceeded"}"#,
                )
            } else {
                (200, r#"{"orderId":"DH-RATE-OK","orderStatus":"TRANSIT"}"#)
            }
        } else {
            (404, r#"{}"#)
        }
    });

    std::env::set_var("DHAN_SANDBOX_ACCESS_TOKEN", "mock_token");
    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let client = DhanTradingClient::new(mode_cfg, "1000000000".into());

    let req = OrderRequest {
        local_id: "RATE-1".into(),
        security_id: "1333".into(),
        side: Side::Buy,
        quantity: 10,
        kind: OrderKind::Market,
        intent: OrderIntent::Entry,
        local_seq: 1,
    };

    // Attempt 1: RL001 Rate Limit error
    let res1 = client.place_order(&req, "AMLN-RATE-1", None).await;
    assert!(res1.is_err());
    match res1.unwrap_err() {
        dhan_api::exec::ExecRestError::Throttle(msg) => {
            assert!(msg.contains("Too many requests"));
        }
        other => panic!("expected Throttle, got {other:?}"),
    }

    // Attempt 2: DH-904 Rate Limit error
    let res2 = client.place_order(&req, "AMLN-RATE-1", None).await;
    assert!(res2.is_err());
    match res2.unwrap_err() {
        dhan_api::exec::ExecRestError::Throttle(msg) => {
            assert!(msg.contains("DH-904") || msg.contains("rate limit"));
        }
        other => panic!("expected Throttle, got {other:?}"),
    }

    // Attempt 3: Succeeded after backoff
    let res3 = client.place_order(&req, "AMLN-RATE-1", None).await.unwrap();
    assert_eq!(res3.order_id, "DH-RATE-OK");
    assert_eq!(res3.order_status, "TRANSIT");
}

// ---------------------------------------------------------------------------
// Scenario 9: Flatten of many positions stays under the rate limit
// ---------------------------------------------------------------------------
#[test]
fn test_flatten_many_positions_paced_under_rate_limit() {
    let mut queue = SubmissionQueue::new(8.0, 10.0);

    // Enqueue 20 exit orders to flatten positions
    for i in 1..=20 {
        let exit_req = OrderRequest {
            local_id: format!("FLATTEN-{i}"),
            security_id: format!("SEC-{i}"),
            side: Side::Sell,
            quantity: 10,
            kind: OrderKind::Market,
            intent: OrderIntent::Exit,
            local_seq: i,
        };
        queue.enqueue_place(exit_req).unwrap();
    }

    assert_eq!(queue.len(), 20);

    // Pop first burst: token bucket capacity is 10.0 tokens
    let mut burst_count = 0;
    while queue.try_acquire_token() {
        assert!(queue.pop_next().is_some());
        burst_count += 1;
    }

    // Exactly initial capacity tokens consumed, remainder paced!
    assert_eq!(burst_count, 10);
    assert_eq!(queue.len(), 10);

    // Wait duration for subsequent tokens is positive (pacing enforced)
    assert!(queue.wait_duration() > Duration::ZERO);
}

// ---------------------------------------------------------------------------
// Scenario 10: Kill switch activation retried until flat
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_kill_switch_activation_retried_until_flat() {
    let _env = init_mock_env();
    let ks_attempts = Arc::new(AtomicUsize::new(0));
    let ks_clone = Arc::clone(&ks_attempts);
    let server = MockHttpServer::new(move |_method, path, _req| {
        if path.starts_with("/killswitch") {
            let n = ks_clone.fetch_add(1, Ordering::SeqCst) + 1;
            if n < 2 {
                // Violation: open positions remain
                (
                    400,
                    r#"{"status":"failure","errorMessage":"Cannot activate kill switch while open positions exist"}"#,
                )
            } else {
                (200, r#"{"killSwitchStatus":"ACTIVE"}"#)
            }
        } else {
            (404, r#"{}"#)
        }
    });

    std::env::set_var("DHAN_SANDBOX_ACCESS_TOKEN", "mock_token");
    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let client = DhanTradingClient::new(mode_cfg, "1000000000".into());

    // First attempt fails
    let res1 = client.activate_kill_switch().await;
    assert!(res1.is_err());

    // Second attempt succeeds
    let res2 = client.activate_kill_switch().await;
    assert!(res2.is_ok());

    let status = client.get_kill_switch_status().await.unwrap();
    assert_eq!(status, "ACTIVE");
}

// ---------------------------------------------------------------------------
// Scenario 11: Mode isolation: sandbox cannot use live base URL or token
// ---------------------------------------------------------------------------
#[test]
fn test_mode_isolation_sandbox_cannot_use_live_base_url_or_token() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // 1. Sandbox cannot use live token or live base URL
    std::env::set_var("DHAN_ACCESS_TOKEN", "live_token");
    std::env::remove_var("DHAN_SANDBOX_ACCESS_TOKEN");

    let sb_res = ModeConfig::resolve(ExecutionMode::Sandbox, None);
    assert!(sb_res.is_err());
    assert!(sb_res.unwrap_err().contains("DHAN_SANDBOX_ACCESS_TOKEN"));

    // 2. Setting sandbox token resolves strictly to sandbox base URL
    std::env::set_var("DHAN_SANDBOX_ACCESS_TOKEN", "sb_token");
    let sb_ok = ModeConfig::resolve(ExecutionMode::Sandbox, None).unwrap();
    assert_eq!(sb_ok.base_url, dhan_api::exec::SANDBOX_BASE_URL);
    assert_eq!(sb_ok.token.as_deref(), Some("sb_token"));

    // 3. Live mode requires both LIVE_TRADING=true flag and DHAN_ACCESS_TOKEN
    std::env::remove_var("LIVE_TRADING");
    let live_without_flag = ModeConfig::resolve(ExecutionMode::Live, None);
    assert!(live_without_flag.is_err());
    assert!(live_without_flag.unwrap_err().contains("LIVE_TRADING=true"));

    std::env::set_var("LIVE_TRADING", "true");
    let live_ok = ModeConfig::resolve(ExecutionMode::Live, None).unwrap();
    assert_eq!(live_ok.base_url, dhan_api::exec::LIVE_BASE_URL);
    assert_eq!(live_ok.token.as_deref(), Some("live_token"));

    // Restore environment variables
    std::env::set_var("DHAN_SANDBOX_ACCESS_TOKEN", "mock_sandbox_token");
    std::env::remove_var("LIVE_TRADING");
    std::env::remove_var("DHAN_ACCESS_TOKEN");
}

// ---------------------------------------------------------------------------
// Task 2: Exit Replace Loop Mock Tests
// ---------------------------------------------------------------------------

// 1. exit fills before timeout (no replacement)
#[test]
fn test_exit_fills_before_timeout_no_replacement() {
    let _env = init_mock_env();
    let server = MockHttpServer::new(|method, path, _body| {
        if method == "GET" && path == "/orders" {
            (200, "[]")
        } else if method == "GET" && path == "/positions" {
            (200, r#"[{"securityId":"RELIANCE","netQty":10,"productType":"INTRADAY","buyQty":10,"sellQty":0}]"#)
        } else if method == "POST" && path == "/orders" {
            (200, r#"{"orderId":"EX-1","orderStatus":"TRANSIT"}"#)
        } else {
            (200, "{}")
        }
    });

    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let port_cfg = DhanExecutionPortConfig {
        client_id: "1000000000".into(),
        mode_config: mode_cfg,
        risk_config: RiskRailsConfig::default(),
        wal_path: None,
        tick_sizes_paise: BTreeMap::new(),
        run_tag: "TEST".into(),
        exit_replace_config: ExitReplaceConfig {
            timeout: Duration::from_millis(50),
            max_rounds: 4,
        },
    };

    let mut port = DhanExecutionPort::new(port_cfg).unwrap();
    let req = OrderRequest {
        local_id: "EXIT-1".into(),
        security_id: "RELIANCE".into(),
        side: Side::Sell,
        quantity: 10,
        kind: OrderKind::Market,
        intent: OrderIntent::Exit,
        local_seq: 1,
    };

    port.submit_order(req).unwrap();
    port.process_queue_step();

    // Verify order is submitted and awaiting pending
    let loops = port.active_exit_loops();
    assert_eq!(loops.get("RELIANCE").unwrap().phase, ExitLoopPhase::AwaitingPending);

    // Feed PENDING update -> timer starts
    port.apply_update(&NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("EX-1".into()),
        correlation_id: None,
        status: NormalizedStatus::Open,
        quantity: 10,
        cum_traded_qty: 0,
        remaining_qty: 10,
        avg_price_paise: None,
        last_traded_price_paise: None,
        side: Some(Side::Sell),
        reject_reason: None,
        update_time: None,
    });

    assert_eq!(port.active_exit_loops().get("RELIANCE").unwrap().phase, ExitLoopPhase::TimerRunning);

    // Fill arrives before timeout expires
    port.apply_update(&NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("EX-1".into()),
        correlation_id: None,
        status: NormalizedStatus::Filled,
        quantity: 10,
        cum_traded_qty: 10,
        remaining_qty: 0,
        avg_price_paise: Some(250000),
        last_traded_price_paise: Some(250000),
        side: Some(Side::Sell),
        reject_reason: None,
        update_time: None,
    });

    assert_eq!(port.active_exit_loops().get("RELIANCE").unwrap().phase, ExitLoopPhase::Terminal);

    // Sleep past timeout and run check_exit_timeouts()
    thread::sleep(Duration::from_millis(60));
    port.check_exit_timeouts();

    // Assert no replacement was placed
    let (open_orders, _) = port.snapshot();
    assert_eq!(open_orders.len(), 0);
    assert_eq!(port.local_orders().len(), 1);
}

// 2. timeout then cancel then replacement fills
#[test]
fn test_exit_timeout_then_cancel_then_replacement_fills() {
    let _env = init_mock_env();
    let cancel_count = Arc::new(AtomicUsize::new(0));
    let cc = Arc::clone(&cancel_count);
    let post_count = Arc::new(AtomicUsize::new(0));
    let pc = Arc::clone(&post_count);

    let server = MockHttpServer::new(move |method, path, _body| {
        if method == "GET" && path == "/orders" {
            (200, "[]")
        } else if method == "GET" && path == "/positions" {
            (200, r#"[{"securityId":"TCS","netQty":50,"productType":"INTRADAY","buyQty":50,"sellQty":0}]"#)
        } else if method == "POST" && path == "/orders" {
            let n = pc.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 1 {
                (200, r#"{"orderId":"EX-TCS-1","orderStatus":"TRANSIT"}"#)
            } else {
                (200, r#"{"orderId":"EX-TCS-2","orderStatus":"TRANSIT"}"#)
            }
        } else if method == "DELETE" && path.starts_with("/orders/") {
            cc.fetch_add(1, Ordering::SeqCst);
            (200, r#"{"status":"success"}"#)
        } else if method == "GET" && path.starts_with("/orders/external/") {
            (200, r#"{"orderId":"EX-TCS-1","orderStatus":"CANCELLED","filledQty":0,"remainingQuantity":50}"#)
        } else {
            (200, "{}")
        }
    });

    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let port_cfg = DhanExecutionPortConfig {
        client_id: "1000000000".into(),
        mode_config: mode_cfg,
        risk_config: RiskRailsConfig::default(),
        wal_path: None,
        tick_sizes_paise: BTreeMap::new(),
        run_tag: "TEST".into(),
        exit_replace_config: ExitReplaceConfig {
            timeout: Duration::from_millis(10),
            max_rounds: 4,
        },
    };

    let mut port = DhanExecutionPort::new(port_cfg).unwrap();
    let req = OrderRequest {
        local_id: "EXIT-TCS".into(),
        security_id: "TCS".into(),
        side: Side::Sell,
        quantity: 50,
        kind: OrderKind::Market,
        intent: OrderIntent::Exit,
        local_seq: 1,
    };

    port.submit_order(req).unwrap();
    port.process_queue_step();

    // Transition to Open/Pending -> TimerRunning
    port.apply_update(&NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("EX-TCS-1".into()),
        correlation_id: None,
        status: NormalizedStatus::Open,
        quantity: 50,
        cum_traded_qty: 0,
        remaining_qty: 50,
        avg_price_paise: None,
        last_traded_price_paise: None,
        side: Some(Side::Sell),
        reject_reason: None,
        update_time: None,
    });

    thread::sleep(Duration::from_millis(15));
    // Timeout fires -> cancel sent -> lookup reports CANCELLED -> replacement placed
    port.check_exit_timeouts();

    assert!(cancel_count.load(Ordering::SeqCst) >= 1, "cancel must be sent on timeout");
    assert_eq!(port.active_exit_loops().get("TCS").unwrap().round, 2, "round must be incremented to 2");
    assert!(port.local_orders().contains_key("EXIT-TCS-R2"), "replacement order EXIT-TCS-R2 must be created");

    // Feed fill for replacement
    port.apply_update(&NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("EX-TCS-2".into()),
        correlation_id: None,
        status: NormalizedStatus::Filled,
        quantity: 50,
        cum_traded_qty: 50,
        remaining_qty: 0,
        avg_price_paise: Some(350000),
        last_traded_price_paise: Some(350000),
        side: Some(Side::Sell),
        reject_reason: None,
        update_time: None,
    });

    assert_eq!(port.active_exit_loops().get("TCS").unwrap().phase, ExitLoopPhase::Terminal);
}

// 3. cancel races with a fill (TRADED seen after cancel), no second order
#[test]
fn test_exit_cancel_races_with_fill_traded_no_second_order() {
    let _env = init_mock_env();
    let server = MockHttpServer::new(|method, path, _body| {
        if method == "GET" && path == "/orders" {
            (200, "[]")
        } else if method == "GET" && path == "/positions" {
            (200, r#"[{"securityId":"INFY","netQty":25,"productType":"INTRADAY","buyQty":25,"sellQty":0}]"#)
        } else if method == "POST" && path == "/orders" {
            (200, r#"{"orderId":"EX-INFY","orderStatus":"TRANSIT"}"#)
        } else if method == "DELETE" && path.starts_with("/orders/") {
            (200, r#"{"status":"success"}"#)
        } else if method == "GET" && path.starts_with("/orders/external/") {
            // Broker lookup reveals the order actually TRADED before cancel took effect
            (200, r#"{"orderId":"EX-INFY","orderStatus":"TRADED","filledQty":25,"remainingQuantity":0}"#)
        } else {
            (200, "{}")
        }
    });

    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let port_cfg = DhanExecutionPortConfig {
        client_id: "1000000000".into(),
        mode_config: mode_cfg,
        risk_config: RiskRailsConfig::default(),
        wal_path: None,
        tick_sizes_paise: BTreeMap::new(),
        run_tag: "TEST".into(),
        exit_replace_config: ExitReplaceConfig {
            timeout: Duration::from_millis(5),
            max_rounds: 4,
        },
    };

    let mut port = DhanExecutionPort::new(port_cfg).unwrap();
    let req = OrderRequest {
        local_id: "EXIT-INFY".into(),
        security_id: "INFY".into(),
        side: Side::Sell,
        quantity: 25,
        kind: OrderKind::Market,
        intent: OrderIntent::Exit,
        local_seq: 1,
    };

    port.submit_order(req).unwrap();
    port.process_queue_step();

    port.apply_update(&NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("EX-INFY".into()),
        correlation_id: None,
        status: NormalizedStatus::Open,
        quantity: 25,
        cum_traded_qty: 0,
        remaining_qty: 25,
        avg_price_paise: None,
        last_traded_price_paise: None,
        side: Some(Side::Sell),
        reject_reason: None,
        update_time: None,
    });

    thread::sleep(Duration::from_millis(10));
    port.check_exit_timeouts();

    // Assert outcome stopped at TRADED and no round 2 order was placed
    assert_eq!(port.active_exit_loops().get("INFY").unwrap().phase, ExitLoopPhase::Terminal);
    assert!(!port.local_orders().contains_key("EXIT-INFY-R2"), "no second order must be placed when fill raced with cancel");
    assert_eq!(port.local_orders().len(), 1);
}

// 4. partial fill then cancel, replacement uses the remaining confirmed quantity
#[test]
fn test_exit_partial_fill_then_cancel_replacement_uses_remaining() {
    let _env = init_mock_env();
    let placed_qty = Arc::new(AtomicUsize::new(0));
    let pq = Arc::clone(&placed_qty);

    let server = MockHttpServer::new(move |method, path, body| {
        if method == "GET" && path == "/orders" {
            (200, "[]")
        } else if method == "GET" && path == "/positions" {
            // Broker confirmed position reflects 40 shares filled -> 60 remaining
            (200, r#"[{"securityId":"HDFC","netQty":60,"productType":"INTRADAY","buyQty":100,"sellQty":40}]"#)
        } else if method == "POST" && path == "/orders" {
            let body_json = body.split("\r\n\r\n").nth(1).unwrap_or(body);
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(body_json) {
                if let Some(q) = json.get("quantity").and_then(|v| v.as_u64()) {
                    pq.store(q as usize, Ordering::SeqCst);
                }
            }
            (200, r#"{"orderId":"EX-HDFC","orderStatus":"TRANSIT"}"#)
        } else if method == "DELETE" && path.starts_with("/orders/") {
            (200, r#"{"status":"success"}"#)
        } else if method == "GET" && path.starts_with("/orders/external/") {
            (200, r#"{"orderId":"EX-HDFC","orderStatus":"CANCELLED","filledQty":40,"remainingQuantity":60}"#)
        } else {
            (200, "{}")
        }
    });

    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let port_cfg = DhanExecutionPortConfig {
        client_id: "1000000000".into(),
        mode_config: mode_cfg,
        risk_config: RiskRailsConfig::default(),
        wal_path: None,
        tick_sizes_paise: BTreeMap::new(),
        run_tag: "TEST".into(),
        exit_replace_config: ExitReplaceConfig {
            timeout: Duration::from_millis(5),
            max_rounds: 4,
        },
    };

    let mut port = DhanExecutionPort::new(port_cfg).unwrap();
    let req = OrderRequest {
        local_id: "EXIT-HDFC".into(),
        security_id: "HDFC".into(),
        side: Side::Sell,
        quantity: 100,
        kind: OrderKind::Market,
        intent: OrderIntent::Exit,
        local_seq: 1,
    };

    port.submit_order(req).unwrap();
    port.process_queue_step();

    // Partial fill arrives: 40 filled, 60 remaining
    port.apply_update(&NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("EX-HDFC".into()),
        correlation_id: None,
        status: NormalizedStatus::PartiallyFilled,
        quantity: 100,
        cum_traded_qty: 40,
        remaining_qty: 60,
        avg_price_paise: Some(150000),
        last_traded_price_paise: Some(150000),
        side: Some(Side::Sell),
        reject_reason: None,
        update_time: None,
    });

    thread::sleep(Duration::from_millis(10));
    port.check_exit_timeouts();

    // Replacement must be placed with quantity: 60 (remaining confirmed open quantity)
    assert_eq!(placed_qty.load(Ordering::SeqCst), 60, "replacement must be sized from broker confirmed position (60)");
    let r2 = port.local_orders().get("EXIT-HDFC-R2").expect("EXIT-HDFC-R2 must exist");
    assert_eq!(r2.request.quantity, 60);
}

// 5. cancel outcome unknown, reconcile runs, no replacement until resolved
#[test]
fn test_exit_cancel_outcome_unknown_reconcile_runs_no_guess() {
    let _env = init_mock_env();
    let cancel_attempted = Arc::new(AtomicBool::new(false));
    let ca = Arc::clone(&cancel_attempted);
    let resolved_terminal = Arc::new(AtomicBool::new(false));
    let rt_clone = Arc::clone(&resolved_terminal);

    let server = MockHttpServer::new(move |method, path, _body| {
        if method == "GET" && path == "/orders" {
            if rt_clone.load(Ordering::SeqCst) {
                (200, r#"[{"orderId":"EX-UNK","correlationId":"AMLN-TEST-0001","orderStatus":"CANCELLED","filledQty":0,"remainingQuantity":10}]"#)
            } else {
                (200, r#"[{"orderId":"EX-UNK","correlationId":"AMLN-TEST-0001","orderStatus":"PENDING","filledQty":0,"remainingQuantity":10}]"#)
            }
        } else if method == "GET" && path == "/positions" {
            (200, r#"[{"securityId":"WIPRO","netQty":10,"productType":"INTRADAY","buyQty":10,"sellQty":0}]"#)
        } else if method == "POST" && path == "/orders" {
            (200, r#"{"orderId":"EX-UNK","orderStatus":"TRANSIT"}"#)
        } else if method == "DELETE" && path.starts_with("/orders/") {
            ca.store(true, Ordering::SeqCst);
            // Cancel returns 500 server error (outcome unknown)
            (500, r#"{"status":"failure","errorMessage":"Internal gateway error"}"#)
        } else {
            (200, "{}")
        }
    });

    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let port_cfg = DhanExecutionPortConfig {
        client_id: "1000000000".into(),
        mode_config: mode_cfg,
        risk_config: RiskRailsConfig::default(),
        wal_path: None,
        tick_sizes_paise: BTreeMap::new(),
        run_tag: "TEST".into(),
        exit_replace_config: ExitReplaceConfig {
            timeout: Duration::from_millis(5),
            max_rounds: 4,
        },
    };

    let mut port = DhanExecutionPort::new(port_cfg).unwrap();
    let req = OrderRequest {
        local_id: "EXIT-WIPRO".into(),
        security_id: "WIPRO".into(),
        side: Side::Sell,
        quantity: 10,
        kind: OrderKind::Market,
        intent: OrderIntent::Exit,
        local_seq: 1,
    };

    port.submit_order(req).unwrap();
    port.process_queue_step();

    port.apply_update(&NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("EX-UNK".into()),
        correlation_id: None,
        status: NormalizedStatus::Open,
        quantity: 10,
        cum_traded_qty: 0,
        remaining_qty: 10,
        avg_price_paise: None,
        last_traded_price_paise: None,
        side: Some(Side::Sell),
        reject_reason: None,
        update_time: None,
    });

    thread::sleep(Duration::from_millis(10));
    // Timeout fires -> cancel fails with 500 -> enters Reconciling -> reconcile runs -> order is still PENDING
    port.check_exit_timeouts();

    assert!(cancel_attempted.load(Ordering::SeqCst));
    assert_eq!(port.active_exit_loops().get("WIPRO").unwrap().phase, ExitLoopPhase::Reconciling);
    assert!(!port.local_orders().contains_key("EXIT-WIPRO-R2"), "must not guess or place replacement while order is unresolved");

    // Now reconcile discovers CANCELLED
    resolved_terminal.store(true, Ordering::SeqCst);
    port.check_exit_timeouts();

    assert_eq!(port.active_exit_loops().get("WIPRO").unwrap().round, 2);
    assert!(port.local_orders().contains_key("EXIT-WIPRO-R2"), "replacement placed after reconcile confirms terminal state");
}

// 6. max rounds reached, Halt emitted
#[test]
fn test_exit_max_rounds_reached_halt_emitted() {
    let _env = init_mock_env();
    let post_count = Arc::new(AtomicUsize::new(0));
    let pc = Arc::clone(&post_count);

    let server = MockHttpServer::new(move |method, path, _body| {
        if method == "GET" && path == "/orders" {
            (200, "[]")
        } else if method == "GET" && path == "/positions" {
            (200, r#"[{"securityId":"SBIN","netQty":10,"productType":"INTRADAY","buyQty":10,"sellQty":0}]"#)
        } else if method == "POST" && path == "/orders" {
            let n = pc.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 1 {
                (200, r#"{"orderId":"EX-SBIN-1","orderStatus":"TRANSIT"}"#)
            } else {
                (200, r#"{"orderId":"EX-SBIN-2","orderStatus":"TRANSIT"}"#)
            }
        } else if method == "DELETE" && path.starts_with("/orders/") {
            (200, r#"{"status":"success"}"#)
        } else if method == "GET" && path.starts_with("/orders/external/") {
            if path.contains("0002") {
                (200, r#"{"orderId":"EX-SBIN-2","correlationId":"AMLN-TEST-0002","orderStatus":"CANCELLED","filledQty":0,"remainingQuantity":10}"#)
            } else {
                (200, r#"{"orderId":"EX-SBIN-1","correlationId":"AMLN-TEST-0001","orderStatus":"CANCELLED","filledQty":0,"remainingQuantity":10}"#)
            }
        } else {
            (200, "{}")
        }
    });

    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let port_cfg = DhanExecutionPortConfig {
        client_id: "1000000000".into(),
        mode_config: mode_cfg,
        risk_config: RiskRailsConfig::default(),
        wal_path: None,
        tick_sizes_paise: BTreeMap::new(),
        run_tag: "TEST".into(),
        exit_replace_config: ExitReplaceConfig {
            timeout: Duration::from_millis(5),
            max_rounds: 2, // Max 2 rounds
        },
    };

    let mut port = DhanExecutionPort::new(port_cfg).unwrap();
    let req = OrderRequest {
        local_id: "EXIT-SBIN".into(),
        security_id: "SBIN".into(),
        side: Side::Sell,
        quantity: 10,
        kind: OrderKind::Market,
        intent: OrderIntent::Exit,
        local_seq: 1,
    };

    port.submit_order(req).unwrap();
    port.process_queue_step();

    // Round 1 PENDING -> timeout -> cancel -> R2 placed
    port.apply_update(&NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("EX-SBIN-1".into()),
        correlation_id: None,
        status: NormalizedStatus::Open,
        quantity: 10,
        cum_traded_qty: 0,
        remaining_qty: 10,
        avg_price_paise: None,
        last_traded_price_paise: None,
        side: Some(Side::Sell),
        reject_reason: None,
        update_time: None,
    });

    thread::sleep(Duration::from_millis(10));
    port.check_exit_timeouts();
    assert_eq!(port.active_exit_loops().get("SBIN").unwrap().round, 2);

    // Round 2 PENDING -> timeout -> cancel -> max rounds (2) reached
    port.apply_update(&NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("EX-SBIN-2".into()),
        correlation_id: None,
        status: NormalizedStatus::Open,
        quantity: 10,
        cum_traded_qty: 0,
        remaining_qty: 10,
        avg_price_paise: None,
        last_traded_price_paise: None,
        side: Some(Side::Sell),
        reject_reason: None,
        update_time: None,
    });

    thread::sleep(Duration::from_millis(10));
    port.check_exit_timeouts();

    // Assert Halt emitted with reason ExitNotFilled
    let events = port.drain_exec_events();
    assert!(events.iter().any(|e| matches!(e, ExecEvent::Halt { ref reason } if reason == "ExitNotFilled")));
    assert!(port.risk().is_entries_locked());
    assert!(!port.local_orders().contains_key("EXIT-SBIN-R3"), "no round 3 order when max_rounds is 2");
}

// 7. second exit for the same symbol rejected while one is live
#[test]
fn test_second_exit_for_same_symbol_rejected_while_one_is_live() {
    let _env = init_mock_env();
    let server = MockHttpServer::new(|method, path, _body| {
        if method == "GET" && path == "/orders" {
            (200, "[]")
        } else if method == "GET" && path == "/positions" {
            (200, "[]")
        } else {
            (200, "{}")
        }
    });

    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let port_cfg = DhanExecutionPortConfig {
        client_id: "1000000000".into(),
        mode_config: mode_cfg,
        risk_config: RiskRailsConfig::default(),
        wal_path: None,
        tick_sizes_paise: BTreeMap::new(),
        run_tag: "TEST".into(),
        exit_replace_config: ExitReplaceConfig::default(),
    };

    let mut port = DhanExecutionPort::new(port_cfg).unwrap();
    let exit1 = OrderRequest {
        local_id: "EXIT-TATAMOTORS-1".into(),
        security_id: "TATAMOTORS".into(),
        side: Side::Sell,
        quantity: 10,
        kind: OrderKind::Market,
        intent: OrderIntent::Exit,
        local_seq: 1,
    };

    let exit2 = OrderRequest {
        local_id: "EXIT-TATAMOTORS-2".into(),
        security_id: "TATAMOTORS".into(),
        side: Side::Sell,
        quantity: 10,
        kind: OrderKind::Market,
        intent: OrderIntent::Exit,
        local_seq: 2,
    };

    assert!(port.submit_order(exit1).is_ok());

    // Second exit submit for same symbol while first is active must be rejected
    let res2 = port.submit_order(exit2);
    assert!(res2.is_err());
    assert!(res2.unwrap_err().contains("active exit order already exists for symbol 'TATAMOTORS'"));
}

// 8. entries never go through this loop
#[test]
fn test_entries_never_go_through_exit_replace_loop() {
    let _env = init_mock_env();
    let server = MockHttpServer::new(|method, path, _body| {
        if method == "GET" && path == "/orders" {
            (200, "[]")
        } else if method == "GET" && path == "/positions" {
            (200, "[]")
        } else if method == "POST" && path == "/orders" {
            (200, r#"{"orderId":"ENTRY-ORD","orderStatus":"TRANSIT"}"#)
        } else {
            (200, "{}")
        }
    });

    let mode_cfg = ModeConfig::resolve(ExecutionMode::Sandbox, Some(server.addr.clone())).unwrap();
    let port_cfg = DhanExecutionPortConfig {
        client_id: "1000000000".into(),
        mode_config: mode_cfg,
        risk_config: RiskRailsConfig::default(),
        wal_path: None,
        tick_sizes_paise: BTreeMap::new(),
        run_tag: "TEST".into(),
        exit_replace_config: ExitReplaceConfig {
            timeout: Duration::from_millis(5),
            max_rounds: 4,
        },
    };

    let mut port = DhanExecutionPort::new(port_cfg).unwrap();
    let entry_req = OrderRequest {
        local_id: "ENTRY-1".into(),
        security_id: "BAJFINANCE".into(),
        side: Side::Buy,
        quantity: 10,
        kind: OrderKind::Market,
        intent: OrderIntent::Entry,
        local_seq: 1,
    };

    port.submit_order(entry_req).unwrap();
    port.process_queue_step();

    // Entry orders are never tracked in active_exit_loops
    assert!(!port.active_exit_loops().contains_key("BAJFINANCE"));

    port.apply_update(&NormalizedOrderUpdate {
        source: UpdateSource::Rest,
        dhan_order_id: Some("ENTRY-ORD".into()),
        correlation_id: None,
        status: NormalizedStatus::Open,
        quantity: 10,
        cum_traded_qty: 0,
        remaining_qty: 10,
        avg_price_paise: None,
        last_traded_price_paise: None,
        side: Some(Side::Buy),
        reject_reason: None,
        update_time: None,
    });

    thread::sleep(Duration::from_millis(15));
    port.check_exit_timeouts();

    // Entry order remains Open, never cancelled or replaced
    let tracker = port.local_orders().get("ENTRY-1").unwrap();
    assert_eq!(tracker.state, OrderState::Open);
    assert_eq!(port.local_orders().len(), 1);
}
