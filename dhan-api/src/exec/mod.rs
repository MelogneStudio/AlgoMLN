pub mod correlation;
pub mod error;
pub mod normalize;
pub mod port;
pub mod queue;
pub mod reconciler;
pub mod rest;
pub mod risk;
pub mod state;
pub mod types;
pub mod wal;
pub mod ws;

pub use correlation::{
    is_valid_correlation_id, CorrelationGenerator, DEFAULT_PREFIX, MAX_CORRELATION_LEN,
};
pub use error::{classify_error, ErrorClass};
pub use normalize::{
    map_status, parse_rest_order, parse_rupees_to_paise, parse_ws_order, rupees_to_paise_half_up,
    NormalizedOrderUpdate, NormalizedStatus, UpdateSource,
};
pub use port::{DhanExecutionPort, DhanExecutionPortConfig};
pub use queue::{
    CounterStatus, Priority, QueueAction, SubmissionQueue, DAILY_ORDER_CAP, STOP_ENTRIES_95_PCT,
    WARN_80_PCT,
};
pub use reconciler::Reconciler;
pub use rest::{
    DhanTradingClient, ExecRestError, PlaceOrderResponse, PnlExitRecord, PositionRecord,
    TradeRecord,
};
pub use risk::{inspect_jwt_expiry, RiskController, RiskRailsConfig};
pub use state::{OrderState, OrderTracker};
pub use types::{
    ExecEvent, ExecutionMode, LocalOrderId, ModeConfig, OrderIntent, OrderKind, OrderRequest,
    Side, LIVE_BASE_URL, LIVE_ORDER_UPDATE_WS, SANDBOX_BASE_URL,
};
pub use wal::{OrderWal, WalRecord};
pub use ws::{spawn_order_update_ws, OrderUpdateWsConfig, WsConnectionEvent};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn test_mode_resolution_isolation() {
        let dry = ModeConfig::resolve(ExecutionMode::Dry, None).expect("dry mode resolves");
        assert_eq!(dry.mode, ExecutionMode::Dry);
        assert!(!dry.live_trading_acknowledged);

        std::env::remove_var("DHAN_SANDBOX_ACCESS_TOKEN");
        let sb_err = ModeConfig::resolve(ExecutionMode::Sandbox, None);
        assert!(sb_err.is_err());
        assert!(sb_err.unwrap_err().contains("DHAN_SANDBOX_ACCESS_TOKEN"));

        std::env::remove_var("LIVE_TRADING");
        std::env::set_var("DHAN_ACCESS_TOKEN", "mock_live_token");
        let live_err = ModeConfig::resolve(ExecutionMode::Live, None);
        assert!(live_err.is_err());
        assert!(live_err.unwrap_err().contains("LIVE_TRADING=true"));

        std::env::set_var("LIVE_TRADING", "true");
        std::env::remove_var("DHAN_ACCESS_TOKEN");
        let live_token_err = ModeConfig::resolve(ExecutionMode::Live, None);
        assert!(live_token_err.is_err());
        assert!(live_token_err.unwrap_err().contains("DHAN_ACCESS_TOKEN"));

        std::env::set_var("DHAN_SANDBOX_ACCESS_TOKEN", "sb_secret");
        let sb_ok = ModeConfig::resolve(ExecutionMode::Sandbox, None).unwrap();
        assert_eq!(sb_ok.base_url, SANDBOX_BASE_URL);
        assert_eq!(sb_ok.token.as_deref(), Some("sb_secret"));

        std::env::set_var("DHAN_ACCESS_TOKEN", "live_secret");
        let live_ok = ModeConfig::resolve(ExecutionMode::Live, None).unwrap();
        assert_eq!(live_ok.base_url, LIVE_BASE_URL);
        assert_eq!(live_ok.token.as_deref(), Some("live_secret"));
        assert!(live_ok.live_trading_acknowledged);

        let mock_override = ModeConfig::resolve(
            ExecutionMode::Sandbox,
            Some("http://127.0.0.1:9999/v2".into()),
        )
        .unwrap();
        assert_eq!(mock_override.base_url, "http://127.0.0.1:9999/v2");
    }

    #[test]
    fn test_error_classification_all_specs() {
        assert_eq!(
            classify_error(Some(401), None, None, None),
            ErrorClass::FatalAuth
        );
        assert_eq!(
            classify_error(None, Some("DH-901"), None, None),
            ErrorClass::FatalAuth
        );
        assert_eq!(
            classify_error(None, Some("DH-902"), None, None),
            ErrorClass::FatalConfig
        );
        assert_eq!(
            classify_error(None, Some("DH-903"), None, None),
            ErrorClass::FatalAccount
        );
        assert_eq!(
            classify_error(Some(429), None, None, None),
            ErrorClass::Throttle
        );
        assert_eq!(
            classify_error(None, Some("DH-904"), None, None),
            ErrorClass::Throttle
        );
        assert_eq!(
            classify_error(None, Some("RL001"), None, None),
            ErrorClass::Throttle
        );
        assert_eq!(
            classify_error(None, None, Some("RATE_LIMIT_ERROR"), None),
            ErrorClass::Throttle
        );
        assert_eq!(
            classify_error(None, Some("DH-905"), None, None),
            ErrorClass::OurBug
        );
        assert_eq!(
            classify_error(None, Some("DH-906"), None, None),
            ErrorClass::BusinessRejection
        );
        assert_eq!(
            classify_error(None, Some("DH-907"), None, None),
            ErrorClass::BadRequest
        );
        assert_eq!(
            classify_error(None, Some("DH-908"), None, None),
            ErrorClass::TransientServerError
        );
        assert_eq!(
            classify_error(None, Some("DH-909"), None, None),
            ErrorClass::UnknownOutcome
        );
        assert_eq!(
            classify_error(Some(500), None, None, None),
            ErrorClass::UnknownOutcome
        );
        assert_eq!(
            classify_error(Some(502), None, None, None),
            ErrorClass::UnknownOutcome
        );
        assert_eq!(
            classify_error(None, Some("DH-910"), None, None),
            ErrorClass::Unclassified
        );
        assert_eq!(
            classify_error(None, Some("XYZ-999"), None, None),
            ErrorClass::Unclassified
        );
    }

    #[test]
    fn test_status_mapping_and_casing() {
        assert_eq!(map_status("transit", 0, 10), NormalizedStatus::Submitted);
        assert_eq!(map_status("PENDING", 0, 10), NormalizedStatus::Open);
        assert_eq!(map_status("Pending", 3, 7), NormalizedStatus::PartiallyFilled);
        assert_eq!(map_status("part_traded", 5, 5), NormalizedStatus::PartiallyFilled);
        assert_eq!(map_status("TRADED", 10, 0), NormalizedStatus::Filled);
        assert_eq!(map_status("cancelled", 0, 10), NormalizedStatus::Cancelled);
        assert_eq!(map_status("Rejected", 0, 10), NormalizedStatus::Rejected);
        assert_eq!(map_status("EXPIRED", 0, 10), NormalizedStatus::Expired);
        assert_eq!(map_status("closed", 0, 0), NormalizedStatus::ClosedLogOnly);
        assert_eq!(
            map_status("SOMETHING_NEW", 0, 10),
            NormalizedStatus::Unknown("SOMETHING_NEW".to_string())
        );
    }

    #[test]
    fn test_rest_and_ws_normalization() {
        let rest_json = serde_json::json!({
            "orderId": "11223344",
            "correlationId": "AlgoMLN-1-1",
            "orderStatus": "PART_TRADED",
            "quantity": 100,
            "filledQty": 40,
            "remainingQuantity": 60,
            "averageTradedPrice": 250.75,
            "transactionType": "BUY",
            "updateTime": "2026-11-22 10:15:30"
        });

        let norm_rest = parse_rest_order(&rest_json).unwrap();
        assert_eq!(norm_rest.source, UpdateSource::Rest);
        assert_eq!(norm_rest.dhan_order_id.as_deref(), Some("11223344"));
        assert_eq!(norm_rest.correlation_id.as_deref(), Some("AlgoMLN-1-1"));
        assert_eq!(norm_rest.status, NormalizedStatus::PartiallyFilled);
        assert_eq!(norm_rest.quantity, 100);
        assert_eq!(norm_rest.cum_traded_qty, 40);
        assert_eq!(norm_rest.remaining_qty, 60);
        assert_eq!(norm_rest.avg_price_paise, Some(25075));
        assert_eq!(norm_rest.side, Some(Side::Buy));

        let ws_json = serde_json::json!({
            "Type": "order_alert",
            "Data": {
                "OrderNo": "11223344",
                "CorrelationId": "AlgoMLN-1-1",
                "Status": "Traded",
                "Quantity": 100,
                "TradedQty": 100,
                "RemainingQuantity": 0,
                "AvgTradedPrice": 251.25,
                "TradedPrice": 251.50,
                "TransactionType": "B"
            }
        });

        let norm_ws = parse_ws_order(&ws_json).unwrap();
        assert_eq!(norm_ws.source, UpdateSource::Websocket);
        assert_eq!(norm_ws.dhan_order_id.as_deref(), Some("11223344"));
        assert_eq!(norm_ws.status, NormalizedStatus::Filled);
        assert_eq!(norm_ws.cum_traded_qty, 100);
        assert_eq!(norm_ws.remaining_qty, 0);
        assert_eq!(norm_ws.last_traded_price_paise, Some(25150));
        assert_eq!(norm_ws.side, Some(Side::Buy));
    }

    #[test]
    fn test_order_tracker_lifecycle_and_absorption() {
        let req = OrderRequest {
            local_id: "L1".into(),
            security_id: "1333".into(),
            side: Side::Buy,
            quantity: 50,
            kind: OrderKind::Limit { price_paise: 20000 },
            intent: OrderIntent::Entry,
            local_seq: 1,
        };

        let mut tracker = OrderTracker::new(req, "AlgoMLN-1".into());
        assert_eq!(tracker.state, OrderState::Created);

        tracker.mark_submitting();
        assert_eq!(tracker.state, OrderState::Submitting);

        let sub_ev = tracker.mark_submitted(Some("DH-100".into()));
        assert_eq!(tracker.state, OrderState::Submitted);
        assert_eq!(
            sub_ev,
            ExecEvent::Submitted {
                local_id: "L1".into(),
                dhan_order_id: Some("DH-100".into())
            }
        );

        let open_up = NormalizedOrderUpdate {
            source: UpdateSource::Rest,
            dhan_order_id: Some("DH-100".into()),
            correlation_id: Some("AlgoMLN-1".into()),
            status: NormalizedStatus::Open,
            quantity: 50,
            cum_traded_qty: 0,
            remaining_qty: 50,
            avg_price_paise: None,
            last_traded_price_paise: None,
            side: Some(Side::Buy),
            reject_reason: None,
            update_time: None,
        };
        let evs = tracker.apply_update(&open_up);
        assert_eq!(tracker.state, OrderState::Open);
        assert_eq!(
            evs,
            vec![ExecEvent::Open {
                local_id: "L1".into(),
                dhan_order_id: "DH-100".into()
            }]
        );

        let pf_up = NormalizedOrderUpdate {
            source: UpdateSource::Websocket,
            dhan_order_id: Some("DH-100".into()),
            correlation_id: Some("AlgoMLN-1".into()),
            status: NormalizedStatus::PartiallyFilled,
            quantity: 50,
            cum_traded_qty: 20,
            remaining_qty: 30,
            avg_price_paise: Some(20000),
            last_traded_price_paise: Some(20000),
            side: Some(Side::Buy),
            reject_reason: None,
            update_time: Some("1".into()),
        };
        let evs2 = tracker.apply_update(&pf_up);
        assert_eq!(tracker.state, OrderState::PartiallyFilled);
        assert_eq!(tracker.cum_traded_qty, 20);
        assert_eq!(tracker.remaining_qty, 30);
        assert_eq!(evs2.len(), 1);
        match &evs2[0] {
            ExecEvent::Fill { qty, price_paise, .. } => {
                assert_eq!(*qty, 20);
                assert_eq!(*price_paise, 20000);
            }
            _ => panic!("expected fill event"),
        }

        let evs_dup = tracker.apply_update(&pf_up);
        assert!(evs_dup.is_empty());

        let fill_up = NormalizedOrderUpdate {
            source: UpdateSource::Rest,
            dhan_order_id: Some("DH-100".into()),
            correlation_id: Some("AlgoMLN-1".into()),
            status: NormalizedStatus::Filled,
            quantity: 50,
            cum_traded_qty: 50,
            remaining_qty: 0,
            avg_price_paise: Some(20000),
            last_traded_price_paise: None,
            side: Some(Side::Buy),
            reject_reason: None,
            update_time: Some("2".into()),
        };
        let evs3 = tracker.apply_update(&fill_up);
        assert_eq!(tracker.state, OrderState::Filled);
        assert_eq!(tracker.cum_traded_qty, 50);
        assert_eq!(tracker.remaining_qty, 0);
        assert_eq!(evs3.len(), 2);
        match &evs3[0] {
            ExecEvent::Fill { qty, .. } => assert_eq!(*qty, 30),
            _ => panic!("expected incremental fill"),
        }
        assert_eq!(evs3[1], ExecEvent::Filled { local_id: "L1".into() });

        let late_up = NormalizedOrderUpdate {
            source: UpdateSource::Websocket,
            dhan_order_id: Some("DH-100".into()),
            correlation_id: Some("AlgoMLN-1".into()),
            status: NormalizedStatus::Open,
            quantity: 50,
            cum_traded_qty: 50,
            remaining_qty: 0,
            avg_price_paise: Some(20000),
            last_traded_price_paise: None,
            side: Some(Side::Buy),
            reject_reason: None,
            update_time: None,
        };
        let late_evs = tracker.apply_update(&late_up);
        assert!(late_evs.is_empty());
        assert_eq!(tracker.state, OrderState::Filled);
    }

    #[test]
    fn test_monotonicity_decrease_triggers_unknown() {
        let req = OrderRequest {
            local_id: "L2".into(),
            security_id: "1333".into(),
            side: Side::Buy,
            quantity: 50,
            kind: OrderKind::Market,
            intent: OrderIntent::Entry,
            local_seq: 2,
        };
        let mut tracker = OrderTracker::new(req, "AlgoMLN-2".into());
        tracker.cum_traded_qty = 30;

        let decrease_up = NormalizedOrderUpdate {
            source: UpdateSource::Rest,
            dhan_order_id: Some("DH-200".into()),
            correlation_id: Some("AlgoMLN-2".into()),
            status: NormalizedStatus::PartiallyFilled,
            quantity: 50,
            cum_traded_qty: 20,
            remaining_qty: 30,
            avg_price_paise: None,
            last_traded_price_paise: None,
            side: Some(Side::Buy),
            reject_reason: None,
            update_time: None,
        };
        let evs = tracker.apply_update(&decrease_up);
        assert_eq!(tracker.state, OrderState::Unknown);
        assert_eq!(evs.len(), 1);
        match &evs[0] {
            ExecEvent::Unknown { reason, .. } => {
                assert!(reason.contains("cumulative traded quantity decreased"));
            }
            _ => panic!("expected Unknown event"),
        }
    }

    #[test]
    fn test_correlation_id_format_and_ownership() {
        let gen = CorrelationGenerator::new("RUN1");
        let cid = gen.generate(42);
        assert!(is_valid_correlation_id(&cid));
        assert!(cid.len() <= MAX_CORRELATION_LEN);
        assert!(gen.is_own(&cid));
        assert!(gen.is_own("AlgoMLN-test"));
        assert!(!gen.is_own("EXTERNAL-1234"));

        let long_gen = CorrelationGenerator::new("VERY_LONG_TAG_THAT_EXCEEDS_NORMAL_LENGTH");
        let long_cid = long_gen.generate(99999);
        assert!(is_valid_correlation_id(&long_cid));
        assert!(long_cid.len() <= MAX_CORRELATION_LEN);
    }

    #[test]
    fn test_wal_write_and_recovery() {
        let dir = std::env::temp_dir().join(format!("wal_test_{}", std::process::id()));
        let wal_path = dir.join("test_orders.wal");
        let _ = std::fs::remove_file(&wal_path);

        let mut wal = OrderWal::open(&wal_path).expect("open wal");

        let req1 = OrderRequest {
            local_id: "O1".into(),
            security_id: "1333".into(),
            side: Side::Buy,
            quantity: 10,
            kind: OrderKind::Market,
            intent: OrderIntent::Entry,
            local_seq: 1,
        };
        let mut tracker1 = OrderTracker::new(req1, "AMLN-1-0001".into());
        tracker1.state = OrderState::Submitted;
        tracker1.dhan_order_id = Some("DH-1".into());
        wal.append(&tracker1).unwrap();

        let req2 = OrderRequest {
            local_id: "O2".into(),
            security_id: "1333".into(),
            side: Side::Sell,
            quantity: 10,
            kind: OrderKind::Market,
            intent: OrderIntent::Exit,
            local_seq: 2,
        };
        let mut tracker2 = OrderTracker::new(req2, "AMLN-1-0002".into());
        tracker2.state = OrderState::Filled;
        tracker2.cum_traded_qty = 10;
        wal.append(&tracker2).unwrap();

        let recovered_open = wal.recover_open_orders().unwrap();
        assert_eq!(recovered_open.len(), 1);
        assert_eq!(recovered_open[0].local_id, "O1");
        assert_eq!(recovered_open[0].state, OrderState::Submitted);
        assert_eq!(recovered_open[0].dhan_order_id.as_deref(), Some("DH-1"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_rest_client_dry_mode() {
        let mode_cfg = ModeConfig::resolve(ExecutionMode::Dry, None).unwrap();
        let client = DhanTradingClient::new(mode_cfg, "1000000000".into());

        client.startup_check().await.unwrap();

        let req = OrderRequest {
            local_id: "D1".into(),
            security_id: "1333".into(),
            side: Side::Buy,
            quantity: 25,
            kind: OrderKind::Limit { price_paise: 25055 },
            intent: OrderIntent::Entry,
            local_seq: 1,
        };

        let place_resp = client.place_order(&req, "AMLN-DRY-1", Some(5)).await.unwrap();
        assert_eq!(place_resp.order_id, "DRY-1");
        assert_eq!(place_resp.order_status, "TRANSIT");

        client.cancel_order("DRY-1").await.unwrap();

        let orders = client.get_orders().await.unwrap();
        assert!(orders.is_empty());

        let ks = client.get_kill_switch_status().await.unwrap();
        assert_eq!(ks, "INACTIVE");
    }

    #[test]
    fn test_submission_queue_prioritization() {
        let mut queue = SubmissionQueue::new(8.0, 10.0);

        let entry_req = OrderRequest {
            local_id: "E1".into(),
            security_id: "1333".into(),
            side: Side::Buy,
            quantity: 10,
            kind: OrderKind::Market,
            intent: OrderIntent::Entry,
            local_seq: 1,
        };
        queue.enqueue_place(entry_req).unwrap();

        queue.enqueue_cancel("C1".into(), "DH-1".into());

        let exit_req = OrderRequest {
            local_id: "X1".into(),
            security_id: "1333".into(),
            side: Side::Sell,
            quantity: 10,
            kind: OrderKind::Market,
            intent: OrderIntent::Exit,
            local_seq: 2,
        };
        queue.enqueue_place(exit_req).unwrap();

        match queue.pop_next().unwrap() {
            QueueAction::Place(r) => assert_eq!(r.intent, OrderIntent::Exit),
            _ => panic!("expected Exit first"),
        }

        match queue.pop_next().unwrap() {
            QueueAction::Cancel { local_id, .. } => assert_eq!(local_id, "C1"),
            _ => panic!("expected Cancel second"),
        }

        match queue.pop_next().unwrap() {
            QueueAction::Place(r) => assert_eq!(r.intent, OrderIntent::Entry),
            _ => panic!("expected Entry third"),
        }
    }

    #[test]
    fn test_daily_order_counter_safety_rails() {
        let mut queue = SubmissionQueue::new(8.0, 10.0);
        assert_eq!(queue.counter_status(), CounterStatus::Normal);

        queue.set_daily_count(WARN_80_PCT);
        assert_eq!(queue.counter_status(), CounterStatus::Warning80Pct);
        assert!(queue.can_submit_entry());

        queue.set_daily_count(STOP_ENTRIES_95_PCT);
        assert_eq!(queue.counter_status(), CounterStatus::StopEntries95Pct);
        assert!(!queue.can_submit_entry());

        let entry_req = OrderRequest {
            local_id: "E2".into(),
            security_id: "1333".into(),
            side: Side::Buy,
            quantity: 10,
            kind: OrderKind::Market,
            intent: OrderIntent::Entry,
            local_seq: 3,
        };
        assert!(queue.enqueue_place(entry_req).is_err());

        let exit_req = OrderRequest {
            local_id: "X2".into(),
            security_id: "1333".into(),
            side: Side::Sell,
            quantity: 10,
            kind: OrderKind::Market,
            intent: OrderIntent::Exit,
            local_seq: 4,
        };
        assert!(queue.enqueue_place(exit_req).is_ok());
    }

    #[test]
    fn test_risk_controller_limits_and_entry_lock() {
        let cfg = RiskRailsConfig {
            max_qty_per_order: Some(100),
            max_value_per_order_paise: Some(100_000),
            max_orders_per_minute: Some(10),
            broker_kill_switch_on_breach: true,
            allow_emergency_delete_all_positions: false,
        };
        let mut risk = RiskController::new(cfg);

        let valid_entry = OrderRequest {
            local_id: "V1".into(),
            security_id: "1333".into(),
            side: Side::Buy,
            quantity: 10,
            kind: OrderKind::Limit { price_paise: 5000 },
            intent: OrderIntent::Entry,
            local_seq: 1,
        };
        assert!(risk.validate_order(&valid_entry).is_ok());

        let mut excess_qty = valid_entry.clone();
        excess_qty.quantity = 150;
        assert!(risk.validate_order(&excess_qty).is_err());

        let mut excess_val = valid_entry.clone();
        excess_val.kind = OrderKind::Limit { price_paise: 20000 };
        assert!(risk.validate_order(&excess_val).is_err());

        risk.lock_entries("test lock");
        assert!(risk.is_entries_locked());
        assert!(risk.validate_order(&valid_entry).is_err());

        let exit_req = OrderRequest {
            local_id: "X1".into(),
            security_id: "1333".into(),
            side: Side::Sell,
            quantity: 10,
            kind: OrderKind::Market,
            intent: OrderIntent::Exit,
            local_seq: 2,
        };
        assert!(risk.validate_order(&exit_req).is_ok());
    }

    #[tokio::test]
    async fn test_reconciler_and_max_daily_loss_sequence() {
        let mode_cfg = ModeConfig::resolve(ExecutionMode::Dry, None).unwrap();
        let client = DhanTradingClient::new(mode_cfg, "1000000000".into());
        let mut risk = RiskController::new(RiskRailsConfig::default());
        let reconciler = Reconciler::new(CorrelationGenerator::new("TEST"));

        let mut local_orders = BTreeMap::new();
        let mut local_positions = BTreeMap::new();
        local_positions.insert("1333".to_string(), 50i64);

        let pending_req = OrderRequest {
            local_id: "P1".into(),
            security_id: "1333".into(),
            side: Side::Buy,
            quantity: 20,
            kind: OrderKind::Limit { price_paise: 20000 },
            intent: OrderIntent::Entry,
            local_seq: 1,
        };
        let mut tracker = OrderTracker::new(pending_req, "AMLN-TEST-1".into());
        tracker.state = OrderState::Open;
        tracker.dhan_order_id = Some("DH-100".into());
        local_orders.insert("P1".into(), tracker);

        let events = reconciler
            .execute_max_daily_loss_breach(&client, &mut risk, &mut local_orders, &mut local_positions)
            .await;

        assert!(risk.is_entries_locked());
        assert!(events.iter().any(|e| matches!(e, ExecEvent::Halt { .. })));
        assert!(events.iter().any(|e| matches!(e, ExecEvent::Cancelled { ref local_id, .. } if local_id == "P1")));
        assert!(events.iter().any(|e| matches!(e, ExecEvent::Submitted { ref local_id, .. } if local_id == "BREACH-EXIT-1333")));
    }

    #[test]
    fn test_dhan_execution_port_dry_mode_3c_adapter() {
        use algomln::trader::{ExecutionPort, OrderEvent};

        let port_cfg = DhanExecutionPortConfig {
            client_id: "1000000000".into(),
            mode_config: ModeConfig::resolve(ExecutionMode::Dry, None).unwrap(),
            risk_config: RiskRailsConfig::default(),
            wal_path: None,
            tick_sizes_paise: BTreeMap::new(),
            run_tag: "DRYRUN".into(),
        };

        let mut port = DhanExecutionPort::new(port_cfg).expect("create port");

        // Submit via 3c trait
        let trader_req = algomln::trader::OrderRequest {
            client_order_id: "ORD-001".into(),
            symbol: "RELIANCE".into(),
            side: algomln::trader::Side::Buy,
            qty: 15,
            reason: algomln::trader::OrderReason::Rule("R1".into()),
            ref_price: 2500.0,
            decided_at: 1700000000,
        };

        port.submit(&trader_req).expect("submit succeeds");

        // Drain 3c events: should contain Acked
        let events = port.drain_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], OrderEvent::Acked { id: "ORD-001".into() });

        // Cancel via 3c trait
        port.cancel("ORD-001".into()).expect("cancel succeeds");
        let cancel_evs = port.drain_events();
        assert_eq!(cancel_evs.len(), 1);
        assert_eq!(cancel_evs[0], OrderEvent::Cancelled { id: "ORD-001".into() });
    }
}
