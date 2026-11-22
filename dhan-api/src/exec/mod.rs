pub mod error;
pub mod normalize;
pub mod state;
pub mod types;

pub use error::{classify_error, ErrorClass};
pub use normalize::{
    map_status, parse_rest_order, parse_rupees_to_paise, parse_ws_order, rupees_to_paise_half_up,
    NormalizedOrderUpdate, NormalizedStatus, UpdateSource,
};
pub use state::{OrderState, OrderTracker};
pub use types::{
    ExecEvent, ExecutionMode, LocalOrderId, ModeConfig, OrderIntent, OrderKind, OrderRequest,
    Side, LIVE_BASE_URL, LIVE_ORDER_UPDATE_WS, SANDBOX_BASE_URL,
};

#[cfg(test)]
mod tests {
    use super::*;

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
}
