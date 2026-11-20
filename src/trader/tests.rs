use super::*;
use crate::engine::LevelFrame;

fn candle(ts: i64, close: f64) -> Candle {
    Candle {
        timestamp: ts,
        open: close,
        high: close,
        low: close,
        close,
        volume: 100.0,
    }
}
fn program(src: &str) -> Program {
    crate::compile(src).unwrap()
}
fn frame(ts: i64, entry: &[(&str, bool)], exit: &[(&str, bool)]) -> LevelFrame {
    LevelFrame {
        symbol: "ABC".into(),
        candle_ts: ts,
        warm: true,
        entry: entry.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
        exit: exit.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
    }
}
fn named_frame(
    symbol: &str,
    ts: i64,
    entry: &[(&str, bool)],
    exit: &[(&str, bool)],
) -> LevelFrame {
    let mut f = frame(ts, entry, exit);
    f.symbol = symbol.into();
    f
}
fn trader(src: &str, cash: f64) -> Trader {
    Trader::new(
        &program(src),
        TraderConfig {
            initial_cash: cash,
            instrument_rules: BTreeMap::new(),
        },
    )
    .unwrap()
}
fn fill_buy(t: &mut Trader, id: &str, qty: u64, price: f64) {
    t.on_order_event(OrderEvent::Acked { id: id.into() });
    t.on_order_event(OrderEvent::Filled {
        id: id.into(),
        qty,
        price,
        fees: 0.0,
    });
}

#[test]
fn observe_updates_edges_and_emits_nothing() {
    let mut t = trader("TRADE_IN ABC\nWHEN close > 1\nBUY 2\n", 100.0);
    let f = frame(1, &[("rule_1", true)], &[]);
    assert!(t
        .on_frame(&f, &candle(1, 10.0), Mode::Observe)
        .orders
        .is_empty());
    assert!(t
        .on_frame(
            &frame(2, &[("rule_1", true)], &[]),
            &candle(2, 10.0),
            Mode::Act
        )
        .orders
        .is_empty());
    assert_eq!(
        t.on_frame(
            &frame(3, &[("rule_1", false)], &[]),
            &candle(3, 10.0),
            Mode::Act
        )
        .orders
        .len(),
        0
    );
    assert_eq!(
        t.on_frame(
            &frame(4, &[("rule_1", true)], &[]),
            &candle(4, 10.0),
            Mode::Act
        )
        .orders[0]
            .qty,
        2
    );
}

#[test]
fn pending_cash_and_unacked_orders_reserve_capacity_until_events() {
    let mut t = trader(
        "TRADE_IN ABC\nRISK MAX_ORDERS 1\nWHEN close > 1\nBUY 5\n",
        50.0,
    );
    let out = t.on_frame(
        &frame(1, &[("rule_1", true)], &[]),
        &candle(1, 10.0),
        Mode::Act,
    );
    assert_eq!(out.orders[0].qty, 5);
    assert_eq!(t.cash(), 50.0); // pending orders do not alter the ledger
    let blocked = t.on_frame(
        &frame(2, &[("rule_1", false)], &[]),
        &candle(2, 10.0),
        Mode::Act,
    );
    let again = t.on_frame(
        &frame(3, &[("rule_1", true)], &[]),
        &candle(3, 10.0),
        Mode::Act,
    );
    assert!(blocked.orders.is_empty());
    assert!(matches!(
        again.events[0],
        TraderEvent::RiskBlocked {
            limit: "MAX_ORDERS",
            ..
        }
    ));
    t.on_order_event(OrderEvent::Rejected {
        id: "trader-1".into(),
        reason: "no".into(),
    });
    let after = t.on_frame(
        &frame(4, &[("rule_1", false)], &[]),
        &candle(4, 10.0),
        Mode::Act,
    );
    let emitted = t.on_frame(
        &frame(5, &[("rule_1", true)], &[]),
        &candle(5, 10.0),
        Mode::Act,
    );
    assert!(after.orders.is_empty());
    assert_eq!(emitted.orders.len(), 1);
}

#[test]
fn cash_is_reserved_across_orders_in_a_single_frame() {
    let mut t = trader(
        "TRADE_IN ABC\nWHEN close > 1\nBUY 4\nWHEN close > 1\nBUY 2\n",
        50.0,
    );
    let out = t.on_frame(
        &frame(1, &[("rule_1", true), ("rule_2", true)], &[]),
        &candle(1, 10.0),
        Mode::Act,
    );
    assert_eq!(out.orders.len(), 1);
    assert_eq!(out.orders[0].qty, 4);
    assert!(matches!(
        out.events[1],
        TraderEvent::RiskBlocked { limit: "CASH", .. }
    ));
}

#[test]
fn rupees_to_paise_matches_integer_path_on_ties_and_negatives() {
    // Non-finite guards to zero.
    assert_eq!(rupees_to_paise(f64::NAN), 0);
    assert_eq!(rupees_to_paise(f64::INFINITY), 0);
    // Plain conversions.
    assert_eq!(rupees_to_paise(12.34), 1234);
    assert_eq!(rupees_to_paise(-12.34), -1234);
    // Exact half-paise ties round to even, symmetrically across zero.
    assert_eq!(rupees_to_paise(0.005), 0); // 0.5 paise -> 0 (even)
    assert_eq!(rupees_to_paise(0.015), 2); // 1.5 paise -> 2 (even)
    assert_eq!(rupees_to_paise(0.025), 2); // 2.5 paise -> 2 (even)
    assert_eq!(rupees_to_paise(-0.005), 0);
    assert_eq!(rupees_to_paise(-0.015), -2);
    assert_eq!(rupees_to_paise(-0.025), -2);
    // The float path agrees with the shared decimal path for the same value.
    for v in ["100.005", "100.015", "-100.005", "0.125", "-0.125", "7.00"] {
        let f: f64 = v.parse().unwrap();
        assert_eq!(
            i128::from(rupees_to_paise(f)),
            decimal_scaled(v, 100).unwrap(),
            "mismatch for {v}",
        );
    }
}

#[test]
fn paise_ledger_and_percentage_sizing_use_integer_math() {
    assert_eq!(decimal_rupees_to_paise("12.34"), Some(1234));
    assert_eq!(decimal_scaled("12.34", 100), Some(1234));
    let t = trader("TRADE_IN ABC\nWHEN close > 1\nBUY 50%\n", 100.0);
    assert_eq!(t.cash_paise(), 10_000);
    assert_eq!(
        t.resolve_size(
            &TradeAmount::BuyCapitalPercentage(
                crate::source::NumericLiteral::parse("33.33".into(), Default::default())
                    .unwrap()
            ),
            3.0,
            "ABC",
        ),
        Some(11)
    );
}

#[test]
fn partial_fills_change_ledger_and_pending_exit_guards_stop_loss() {
    let mut t = trader(
        "TRADE_IN ABC\nSTOP_LOSS 10%\nWHEN close > 1\nBUY 10\n",
        1000.0,
    );
    let buy = t
        .on_frame(
            &frame(1, &[("rule_1", true)], &[]),
            &candle(1, 10.0),
            Mode::Act,
        )
        .orders
        .remove(0);
    t.on_order_event(OrderEvent::Acked {
        id: buy.client_order_id.clone(),
    });
    t.on_order_event(OrderEvent::PartialFill {
        id: buy.client_order_id.clone(),
        qty: 4,
        price: 10.0,
        fees: 1.0,
    });
    assert_eq!(t.position("ABC").unwrap().filled_qty, 4);
    assert_eq!(t.cash(), 959.0);
    let exit = t.on_frame(
        &frame(2, &[("rule_1", false)], &[]),
        &candle(2, 8.0),
        Mode::Act,
    );
    assert_eq!(exit.orders.len(), 1);
    assert_eq!(exit.orders[0].qty, 4);
    let next = t.on_frame(
        &frame(3, &[("rule_1", false)], &[]),
        &candle(3, 7.0),
        Mode::Act,
    );
    assert!(next.orders.is_empty());
    t.on_order_event(OrderEvent::Rejected {
        id: exit.orders[0].client_order_id.clone(),
        reason: "rejected".into(),
    });
    let rearmed = t.on_frame(
        &frame(4, &[("rule_1", false)], &[]),
        &candle(4, 7.0),
        Mode::Act,
    );
    assert_eq!(rearmed.orders[0].reason, OrderReason::StopLoss);
}

#[test]
fn max_daily_loss_requires_cash_and_blocks_at_threshold() {
    let p=program("TRADE_IN ABC\nRISK MAX_DAILY_LOSS 5%\nWHEN close > 1\nBUY 10\nWHEN close < 1\nSELL ALL\n");
    assert_eq!(
        Trader::new(&p, TraderConfig::default()).unwrap_err(),
        TraderError::InitialCashRequired
    );
    let mut t = Trader::new(
        &p,
        TraderConfig {
            initial_cash: 100.0,
            instrument_rules: BTreeMap::new(),
        },
    )
    .unwrap();
    let buy = t
        .on_frame(
            &frame(1, &[("rule_1", true)], &[("rule_2", false)]),
            &candle(1, 10.0),
            Mode::Act,
        )
        .orders
        .remove(0);
    fill_buy(&mut t, &buy.client_order_id, 10, 10.0);
    let sell = t
        .on_frame(
            &frame(2, &[("rule_1", false)], &[("rule_2", true)]),
            &candle(2, 9.0),
            Mode::Act,
        )
        .orders
        .remove(0);
    t.on_order_event(OrderEvent::Acked {
        id: sell.client_order_id.clone(),
    });
    t.on_order_event(OrderEvent::Filled {
        id: sell.client_order_id,
        qty: 10,
        price: 9.5,
        fees: 0.0,
    });
    assert_eq!(t.realized_pnl_today(), -5.0);
    assert_eq!(t.realized_loss_today(), 5.0);
    let blocked = t.on_frame(
        &frame(3, &[("rule_1", true)], &[("rule_2", false)]),
        &candle(3, 10.0),
        Mode::Act,
    );
    assert!(matches!(
        blocked.events[0],
        TraderEvent::RiskBlocked {
            limit: "MAX_DAILY_LOSS",
            ..
        }
    ));
    t.start_day(4);
    assert_eq!(t.realized_pnl_today(), 0.0);
    assert_eq!(t.realized_loss_today(), 0.0);
}

#[test]
fn daily_loss_breach_cancels_entries_flattens_and_catches_late_fills() {
    let mut t = trader(
        "TRADE_IN ABC\nRISK MAX_DAILY_LOSS 1%\nWHEN close > 4\nBUY 10\nWHEN close > 4\nBUY 2\nWHEN close < 6\nSELL 1\n",
        100.0,
    );
    let entries = t.on_frame(
        &frame(
            1,
            &[("rule_1", true), ("rule_2", true)],
            &[("rule_3", false)],
        ),
        &candle(1, 5.0),
        Mode::Act,
    );
    assert_eq!(entries.orders.len(), 2);
    t.on_order_event(OrderEvent::Acked {
        id: entries.orders[0].client_order_id.clone(),
    });
    t.on_order_event(OrderEvent::Filled {
        id: entries.orders[0].client_order_id.clone(),
        qty: 10,
        price: 5.0,
        fees: 0.0,
    });

    let sell = t
        .on_frame(
            &frame(
                2,
                &[("rule_1", false), ("rule_2", false)],
                &[("rule_3", true)],
            ),
            &candle(2, 4.0),
            Mode::Act,
        )
        .orders
        .remove(0);
    t.on_order_event(OrderEvent::Acked {
        id: sell.client_order_id.clone(),
    });
    t.on_order_event(OrderEvent::Filled {
        id: sell.client_order_id,
        qty: 1,
        price: 4.0,
        fees: 0.0,
    });
    assert_eq!(t.realized_pnl_today(), -1.0);

    let breach = t.on_frame(
        &frame(
            3,
            &[("rule_1", true), ("rule_2", false)],
            &[("rule_3", false)],
        ),
        &candle(3, 4.0),
        Mode::Act,
    );
    assert_eq!(breach.cancels, vec!["trader-2"]);
    assert_eq!(breach.orders.len(), 1);
    assert_eq!(breach.orders[0].qty, 9);
    assert_eq!(breach.orders[0].reason, OrderReason::DailyLossFlatten);
    assert!(breach.events.iter().any(|e| matches!(
        e,
        TraderEvent::RiskBlocked {
            limit: "MAX_DAILY_LOSS",
            ..
        }
    )));

    // A fill racing the cancellation is booked; the still-active breach
    // causes another flatten after the first flatten completes.
    t.on_order_event(OrderEvent::Acked {
        id: "trader-2".into(),
    });
    t.on_order_event(OrderEvent::Filled {
        id: "trader-2".into(),
        qty: 2,
        price: 4.0,
        fees: 0.0,
    });
    t.on_order_event(OrderEvent::Acked {
        id: breach.orders[0].client_order_id.clone(),
    });
    t.on_order_event(OrderEvent::Filled {
        id: breach.orders[0].client_order_id.clone(),
        qty: 9,
        price: 4.0,
        fees: 0.0,
    });
    let second_flatten = t.on_frame(
        &frame(
            4,
            &[("rule_1", false), ("rule_2", false)],
            &[("rule_3", false)],
        ),
        &candle(4, 4.0),
        Mode::Act,
    );
    assert_eq!(
        second_flatten.orders[0].reason,
        OrderReason::DailyLossFlatten
    );
    assert_eq!(second_flatten.orders[0].qty, 2);

    t.start_day(5);
    let _ = t.on_frame(
        &frame(
            5,
            &[("rule_1", false), ("rule_2", false)],
            &[("rule_3", false)],
        ),
        &candle(5, 4.0),
        Mode::Act,
    );
    let next_day = t.on_frame(
        &frame(
            6,
            &[("rule_1", true), ("rule_2", false)],
            &[("rule_3", false)],
        ),
        &candle(6, 5.0),
        Mode::Act,
    );
    assert!(!next_day.orders.is_empty());
}

#[test]
fn daily_loss_flattens_other_portfolio_symbols_at_their_latest_close() {
    let mut t = trader(
        "TRADE_IN ABC,XYZ\nRISK MAX_DAILY_LOSS 1%\nWHEN close > 4\nBUY 1\nWHEN close < 10\nSELL 1\n",
        100.0,
    );
    let abc_buy = t
        .on_frame(
            &named_frame("ABC", 1, &[("rule_1", true)], &[("rule_2", false)]),
            &candle(1, 10.0),
            Mode::Act,
        )
        .orders
        .remove(0);
    t.on_order_event(OrderEvent::Acked {
        id: abc_buy.client_order_id.clone(),
    });
    t.on_order_event(OrderEvent::Filled {
        id: abc_buy.client_order_id,
        qty: 1,
        price: 10.0,
        fees: 0.0,
    });
    let xyz_buy = t
        .on_frame(
            &named_frame("XYZ", 2, &[("rule_1", true)], &[("rule_2", false)]),
            &candle(2, 12.0),
            Mode::Act,
        )
        .orders
        .remove(0);
    t.on_order_event(OrderEvent::Acked {
        id: xyz_buy.client_order_id.clone(),
    });
    t.on_order_event(OrderEvent::Filled {
        id: xyz_buy.client_order_id,
        qty: 1,
        price: 12.0,
        fees: 0.0,
    });
    let abc_sell = t
        .on_frame(
            &named_frame("ABC", 3, &[("rule_1", false)], &[("rule_2", true)]),
            &candle(3, 9.0),
            Mode::Act,
        )
        .orders
        .remove(0);
    t.on_order_event(OrderEvent::Acked {
        id: abc_sell.client_order_id.clone(),
    });
    t.on_order_event(OrderEvent::Filled {
        id: abc_sell.client_order_id,
        qty: 1,
        price: 9.0,
        fees: 0.0,
    });

    let breached = t.on_frame(
        &named_frame("ABC", 4, &[("rule_1", false)], &[("rule_2", false)]),
        &candle(4, 8.0),
        Mode::Act,
    );
    assert_eq!(breached.orders.len(), 1);
    assert_eq!(breached.orders[0].symbol, "XYZ");
    assert_eq!(breached.orders[0].ref_price, 12.0);
    assert_eq!(breached.orders[0].reason, OrderReason::DailyLossFlatten);
}

#[test]
fn threshold_remains_breached_after_later_profit_offsets_net_pnl() {
    let mut t = trader(
        "TRADE_IN ABC\nRISK MAX_DAILY_LOSS 5%\nWHEN close > 1\nBUY 10\nWHEN close < 1\nSELL 5\nWHEN close < 1\nSELL 5\n",
        100.0,
    );
    let buy = t
        .on_frame(
            &frame(
                1,
                &[("rule_1", true)],
                &[("rule_2", false), ("rule_3", false)],
            ),
            &candle(1, 10.0),
            Mode::Act,
        )
        .orders
        .remove(0);
    fill_buy(&mut t, &buy.client_order_id, 10, 10.0);
    let exits = t.on_frame(
        &frame(
            2,
            &[("rule_1", false)],
            &[("rule_2", true), ("rule_3", true)],
        ),
        &candle(2, 9.0),
        Mode::Act,
    );
    assert_eq!(exits.orders.len(), 2);
    t.on_order_event(OrderEvent::Acked {
        id: exits.orders[0].client_order_id.clone(),
    });
    t.on_order_event(OrderEvent::Filled {
        id: exits.orders[0].client_order_id.clone(),
        qty: 5,
        price: 9.0,
        fees: 0.0,
    });
    t.on_order_event(OrderEvent::Acked {
        id: exits.orders[1].client_order_id.clone(),
    });
    t.on_order_event(OrderEvent::Filled {
        id: exits.orders[1].client_order_id.clone(),
        qty: 5,
        price: 11.0,
        fees: 0.0,
    });
    assert_eq!(t.realized_pnl_today(), 0.0);
    assert_eq!(t.realized_loss_today(), 5.0);
    let later = t.on_frame(
        &frame(
            3,
            &[("rule_1", true)],
            &[("rule_2", false), ("rule_3", false)],
        ),
        &candle(3, 10.0),
        Mode::Act,
    );
    assert!(later.orders.is_empty());
    assert!(later.events.iter().any(|event| matches!(
        event,
        TraderEvent::RiskBlocked {
            limit: "MAX_DAILY_LOSS",
            ..
        }
    )));
}

#[test]
fn sizing_rounds_down_to_lot_and_sl_beats_tp() {
    let src = "TRADE_IN ABC\nSTOP_LOSS 10%\nTAKE_PROFIT 10%\nWHEN close > 1\nBUY 10\n";
    let mut t = Trader::new(
        &program(src),
        TraderConfig {
            initial_cash: 1000.0,
            instrument_rules: BTreeMap::from([(
                "ABC".into(),
                InstrumentRule {
                    lot_size: 3,
                    min_qty: 3,
                },
            )]),
        },
    )
    .unwrap();
    let buy = t
        .on_frame(
            &frame(1, &[("rule_1", true)], &[]),
            &candle(1, 10.0),
            Mode::Act,
        )
        .orders
        .remove(0);
    assert_eq!(buy.qty, 9);
    fill_buy(&mut t, &buy.client_order_id, 9, 10.0);
    let exit = t.on_frame(
        &frame(2, &[("rule_1", false)], &[]),
        &candle(2, 8.0),
        Mode::Act,
    );
    assert_eq!(exit.orders[0].reason, OrderReason::StopLoss);
    assert_eq!(exit.orders[0].qty, 9);
}

#[test]
fn scripted_port_delivers_events_in_injected_order() {
    let mut port = ScriptedPort::default();
    let req = OrderRequest {
        client_order_id: "x".into(),
        symbol: "ABC".into(),
        side: Side::Buy,
        qty: 2,
        reason: OrderReason::Rule("r".into()),
        ref_price: 3.0,
        decided_at: 1,
    };
    port.submit(&req).unwrap();
    port.push(OrderEvent::Acked { id: "x".into() });
    port.push(OrderEvent::Filled {
        id: "x".into(),
        qty: 2,
        price: 3.0,
        fees: 0.0,
    });
    assert_eq!(port.submitted, vec![req]);
    assert!(matches!(
        port.drain_events().as_slice(),
        [OrderEvent::Acked { .. }, OrderEvent::Filled { .. }]
    ));
}

#[test]
fn positions_limit_reserves_symbols_and_exit_rules_use_remaining_net() {
    let mut t = trader(
        "TRADE_IN ABC\nRISK MAX_POSITIONS 1\nRISK MAX_ORDERS 1\nWHEN close > 1\nBUY 10\nWHEN close < 1\nSELL 3\nWHEN close < 1\nSELL 5\n",
        1000.0,
    );
    let buy = t
        .on_frame(
            &frame(
                1,
                &[("rule_1", true)],
                &[("rule_2", false), ("rule_3", false)],
            ),
            &candle(1, 10.0),
            Mode::Act,
        )
        .orders
        .remove(0);
    let reset_entry = t.on_frame(
        &frame(
            2,
            &[("rule_1", false)],
            &[("rule_2", false), ("rule_3", false)],
        ),
        &candle(2, 10.0),
        Mode::Act,
    );
    assert!(reset_entry.orders.is_empty());
    fill_buy(&mut t, &buy.client_order_id, 10, 10.0);
    let exits = t.on_frame(
        &frame(
            3,
            &[("rule_1", true)],
            &[("rule_2", true), ("rule_3", true)],
        ),
        &candle(3, 9.0),
        Mode::Act,
    );
    assert_eq!(exits.orders.len(), 2);
    assert!(matches!(
        exits.events[0],
        TraderEvent::RiskBlocked {
            limit: "MAX_POSITIONS",
            ..
        }
    ));
    assert_eq!(
        exits.orders.iter().map(|o| o.qty).collect::<Vec<_>>(),
        vec![3, 5]
    );
    assert_eq!(t.net_position("ABC"), 2);
}

#[test]
fn fill_without_prior_ack_is_accepted() {
    let mut t = trader("TRADE_IN ABC\nWHEN close > 1\nBUY 5\n", 100.0);
    let out = t.on_frame(
        &frame(1, &[("rule_1", true)], &[]),
        &candle(1, 10.0),
        Mode::Act,
    );
    assert_eq!(out.orders.len(), 1);
    let order_id = &out.orders[0].client_order_id;
    // Deliver Filled directly without sending Acked first
    let events = t.on_order_event(OrderEvent::Filled {
        id: order_id.clone(),
        qty: 5,
        price: 10.0,
        fees: 0.0,
    });
    assert!(events.iter().any(|e| matches!(e, TraderEvent::Fill { qty: 5, .. })));
    assert_eq!(t.net_position("ABC"), 5);
    assert_eq!(t.pending_order_count(), 0);
}

#[test]
fn gap_up_open_does_not_result_in_negative_cash() {
    let mut t = trader("TRADE_IN ABC\nWHEN close > 1\nBUY 10\n", 100.0);
    let out = t.on_frame(
        &frame(1, &[("rule_1", true)], &[]),
        &candle(1, 10.0),
        Mode::Act,
    );
    assert_eq!(out.orders.len(), 1);
    let order_id = &out.orders[0].client_order_id;
    // Order was 10 shares at ref_price 10.0 (total 100.0 cash)
    // Fills at 25.0 (gap up to 25.0, where 10 shares would cost 250.0)
    let events = t.on_order_event(OrderEvent::Filled {
        id: order_id.clone(),
        qty: 10,
        price: 25.0,
        fees: 0.0,
    });
    // With 100.0 cash and 25.0 price, max affordable is 4 shares
    assert!(events.iter().any(|e| matches!(e, TraderEvent::Fill { qty: 4, .. })));
    assert_eq!(t.net_position("ABC"), 4);
    assert!(t.cash_paise() >= 0);
    assert_eq!(t.cash_paise(), 0);
}

#[test]
fn naked_sell_does_not_credit_cash() {
    let mut t = trader("TRADE_IN ABC\nWHEN close > 1\nBUY 5\n", 100.0);
    let initial_cash = t.cash_paise();
    // Simulate receiving a Sell fill directly when position is flat
    t.apply_fill("ABC", Side::Sell, 10, 5000, 0);
    assert_eq!(t.net_position("ABC"), 0);
    assert_eq!(t.cash_paise(), initial_cash);
}
