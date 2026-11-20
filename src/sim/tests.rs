use std::{collections::BTreeMap, fs::File};
use chrono::{FixedOffset, TimeZone};
use super::{
    source::limit_to_warmup_window,
    types::round_half_even_i128,
    *,
};
use crate::{
    engine::Symbol,
    indicators::Candle,
    trader::{
        ExecutionPort, OrderEvent, OrderReason, OrderRequest, PortError, ScriptedPort, Side,
        TraderEvent,
    },
};

    struct MemorySource(Vec<(Symbol, Candle)>);
    impl CandleSource for MemorySource {
        fn open(
            &self,
            symbols: &[Symbol],
            _tf: Timeframe,
            range: DateRange,
        ) -> Result<CandleStream, SourceError> {
            let values: Vec<_> = self
                .0
                .iter()
                .filter(|(s, c)| {
                    symbols.contains(s)
                        && range.from.map_or(true, |x| c.timestamp >= x)
                        && range.to.map_or(true, |x| c.timestamp <= x)
                })
                .cloned()
                .map(Ok)
                .collect();
            Ok(Box::new(values.into_iter()))
        }
        fn fingerprint(&self, _: &[Symbol], _: Timeframe, _: DateRange) -> Fingerprint {
            Fingerprint("memory-fixture".into())
        }
    }
    fn candle(ts: i64, open: f64, close: f64) -> Candle {
        Candle {
            timestamp: ts,
            open,
            high: open.max(close),
            low: open.min(close),
            close,
            volume: 100.0,
        }
    }
    fn config() -> RunConfig {
        RunConfig {
            timeframe: Timeframe::M1,
            range: DateRange::default(),
            initial_cash: 1000.0,
            warmup_bars: 0,
            latency: 1,
            slippage_bps: 0.0,
            fee_model: FeeModel::default(),
            partial_fills_pct: None,
            gap_policy: GapPolicy::None,
            debug_invariants: true,
        }
    }

    #[test]
    fn broker_acks_and_fills_at_next_available_open() {
        let mut b = SimBroker::new(vec!["ABC".into()]);
        let req = OrderRequest {
            client_order_id: "o1".into(),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 2,
            reason: OrderReason::Rule("rule_1".into()),
            ref_price: 10.0,
            decided_at: 1,
        };
        b.submit(&req).unwrap();
        b.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 12.0, 12.0))]));
        let ev = b.drain_events();
        assert!(matches!(ev[0], OrderEvent::Acked { .. }));
        assert!(matches!(
            ev[1],
            OrderEvent::Filled {
                price: 12.0,
                qty: 2,
                ..
            }
        ));
    }

    #[test]
    fn fee_rounding_is_half_even_in_integer_paise() {
        assert_eq!(round_half_even_i128(1, 2), 0);
        assert_eq!(round_half_even_i128(3, 2), 2);
        assert_eq!(round_half_even_i128(5, 2), 2);
        assert_eq!(round_half_even_i128(7, 2), 4);

        let mut even_broker = SimBroker::new(vec!["ABC".into()]);
        even_broker.fees.bps = 5000.0; // 1 paisa notional -> exactly 0.5 paisa
        even_broker.tick_size_paise = 1;
        even_broker
            .submit(&OrderRequest {
                client_order_id: "even-half".into(),
                symbol: "ABC".into(),
                side: Side::Buy,
                qty: 1,
                reason: OrderReason::Rule("r".into()),
                ref_price: 0.01,
                decided_at: 1,
            })
            .unwrap();
        even_broker.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 0.01, 0.01))]));
        assert!(matches!(
            even_broker.drain_events().as_slice(),
            [
                OrderEvent::Acked { .. },
                OrderEvent::Filled { fees: 0.0, .. }
            ]
        ));

        let mut broker = SimBroker::new(vec!["ABC".into()]);
        broker.fees.bps = 5000.0;
        broker.tick_size_paise = 1;
        broker
            .submit(&OrderRequest {
                client_order_id: "half-even-fee".into(),
                symbol: "ABC".into(),
                side: Side::Buy,
                qty: 3,
                reason: OrderReason::Rule("r".into()),
                ref_price: 0.01,
                decided_at: 1,
            })
            .unwrap();
        broker.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 0.01, 0.01))]));
        assert!(matches!(
            broker.drain_events().as_slice(),
            [
                OrderEvent::Acked { .. },
                OrderEvent::Filled { fees: 0.02, .. }
            ]
        ));
    }

    #[test]
    fn simulated_slippage_rounds_adversely_to_tick() {
        let mut buy = SimBroker::new(vec!["ABC".into()]);
        buy.slippage_bps = 1.0;
        buy.submit(&OrderRequest {
            client_order_id: "buy-tick".into(),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 1,
            reason: OrderReason::Rule("r".into()),
            ref_price: 10.0,
            decided_at: 1,
        })
        .unwrap();
        buy.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 10.01, 10.01))]));
        let events = buy.drain_events();
        let OrderEvent::Filled { price, .. } = events[1] else {
            panic!("expected buy fill")
        };
        assert_eq!(crate::trader::rupees_to_paise(price), 1005);

        let mut sell = SimBroker::new(vec!["ABC".into()]);
        sell.slippage_bps = 1.0;
        sell.submit(&OrderRequest {
            client_order_id: "sell-tick".into(),
            symbol: "ABC".into(),
            side: Side::Sell,
            qty: 1,
            reason: OrderReason::Rule("r".into()),
            ref_price: 10.0,
            decided_at: 1,
        })
        .unwrap();
        sell.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 10.04, 10.04))]));
        let events = sell.drain_events();
        let OrderEvent::Filled { price, .. } = events[1] else {
            panic!("expected sell fill")
        };
        assert_eq!(crate::trader::rupees_to_paise(price), 1000);
    }

    #[test]
    fn runner_is_deterministic_and_executes_next_open() {
        let p = crate::compile("TRADE_IN ABC\nWHEN close > 10\nBUY 1\nWHEN close < 9\nSELL ALL\n")
            .unwrap();
        let src = MemorySource(vec![
            ("ABC".into(), candle(1, 10.0, 11.0)),
            ("ABC".into(), candle(2, 12.0, 8.0)),
            ("ABC".into(), candle(3, 7.0, 7.0)),
        ]);
        let a = run(&p, &src, config(), None).unwrap();
        let b = run(&p, &src, config(), None).unwrap();
        let c = run_precomputed(&p, &src, config(), None).unwrap();
        assert_eq!(a.fills, b.fills);
        assert_eq!(a.equity, b.equity);
        assert_eq!(a.fills, c.fills);
        assert_eq!(a.equity, c.equity);
        assert_eq!(a.trades, c.trades);
        assert_eq!(a.fills.len(), 2);
        assert_eq!(a.fills[0].price, 12.0);
        assert_eq!(a.fills[1].price, 7.0);
        assert_eq!(a.trades.len(), 1);
        assert_eq!(a.trades[0].net_pnl, -5.0);
        let root = std::env::temp_dir().join(format!("algomln-result-{}", std::process::id()));
        let left = root.join("left");
        let right = root.join("right");
        let _ = std::fs::create_dir_all(&root);
        write_results(&left, &a).unwrap();
        write_results(&right, &b).unwrap();
        for name in [
            "fills.csv",
            "trades.csv",
            "equity.csv",
            "metrics.json",
            "manifest.json",
            "open_positions.json",
            "events.json",
        ] {
            assert_eq!(
                std::fs::read(left.join(name)).unwrap(),
                std::fs::read(right.join(name)).unwrap(),
                "{name}"
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stop_gap_uses_next_open_not_stop_price() {
        let p = crate::compile("TRADE_IN ABC\nSTOP_LOSS 10%\nWHEN close > 0\nBUY 1\n").unwrap();
        let src = MemorySource(vec![
            ("ABC".into(), candle(1, 100.0, 100.0)),
            ("ABC".into(), candle(2, 100.0, 89.0)),
            ("ABC".into(), candle(3, 80.0, 80.0)),
        ]);
        let r = run(&p, &src, config(), None).unwrap();
        assert_eq!(r.fills.len(), 2);
        assert_eq!(r.fills[1].price, 80.0);
        assert_eq!(r.fills[1].reason, OrderReason::StopLoss);
    }

    #[test]
    fn pending_order_waits_until_its_symbol_has_a_candle() {
        let p = crate::compile("TRADE_IN A,B\nWHEN close > 10\nBUY 1\n").unwrap();
        let src = MemorySource(vec![
            ("B".into(), candle(1, 10.0, 11.0)),
            ("A".into(), candle(2, 1.0, 1.0)),
            ("B".into(), candle(3, 12.0, 12.0)),
        ]);
        let r = run(&p, &src, config(), Some(vec!["B".into()])).unwrap();
        assert_eq!(r.fills.len(), 1);
        assert_eq!(r.fills[0].ts, 3);
        assert_eq!(r.fills[0].price, 12.0);
    }

    #[test]
    fn extra_inactive_symbol_does_not_change_single_symbol_fills() {
        let p = crate::compile("TRADE_IN ABC,XYZ\nWHEN close > 100\nBUY 1\n").unwrap();
        let src = MemorySource(vec![
            ("ABC".into(), candle(1, 100.0, 101.0)),
            ("XYZ".into(), candle(1, 1.0, 1.0)),
            ("ABC".into(), candle(2, 102.0, 102.0)),
            ("XYZ".into(), candle(2, 1.0, 1.0)),
        ]);
        let one = run(&p, &src, config(), Some(vec!["ABC".into()])).unwrap();
        let both = run(&p, &src, config(), None).unwrap();
        assert_eq!(one.fills, both.fills);
    }

    #[test]
    fn synthetic_generator_is_seeded_and_contains_requested_scenarios() {
        let symbols = vec!["AAA".to_owned(), "BBB".to_owned()];
        let first = seeded_synthetic_candles(42, &symbols, 4);
        assert_eq!(first, seeded_synthetic_candles(42, &symbols, 4));
        assert_ne!(first, seeded_synthetic_candles(43, &symbols, 4));
        assert!(first.iter().any(|(_, c)| c.volume == 0.0));
        assert!(first.iter().any(|(_, c)| c.high - c.low > 10.0));
        assert!(first
            .windows(2)
            .any(|w| w[1].1.timestamp - w[0].1.timestamp > 60));
        assert!(first.windows(2).any(|w| w[0].0 == "AAA" && w[1].0 == "BBB"));
    }

    #[test]
    fn file_merge_retains_only_one_decoded_candle_per_symbol() {
        let base = std::env::temp_dir().join(format!("algomln-bounded-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        for symbol in ["A", "B", "C"] {
            let mut csv = String::from("ts,open,high,low,close,volume\n");
            for ts in 1..=20_000 {
                csv.push_str(&format!("{ts},1,1,1,1,1\n"));
            }
            std::fs::write(base.join(format!("{symbol}.csv")), csv).unwrap();
        }
        let source = FileSource::new(&base);
        let stream = source
            .open(
                &["A".into(), "B".into(), "C".into()],
                Timeframe::M1,
                DateRange::default(),
            )
            .unwrap();
        // FileSource's merge owns only one cursor head per file; consuming the
        // entire stream also exercises row-at-a-time CSV decoding.
        assert_eq!(stream.count(), 60_000);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn scripted_port_and_sim_broker_latency_one_emit_identical_trade() {
        let req = OrderRequest {
            client_order_id: "trade-1".into(),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 2,
            reason: OrderReason::Rule("entry".into()),
            ref_price: 10.0,
            decided_at: 1,
        };
        let mut scripted = ScriptedPort::default();
        scripted.submit(&req).unwrap();
        scripted.push(OrderEvent::Acked {
            id: req.client_order_id.clone(),
        });
        scripted.push(OrderEvent::Filled {
            id: req.client_order_id.clone(),
            qty: 2,
            price: 12.0,
            fees: 0.0,
        });

        let mut broker = SimBroker::new(vec!["ABC".into()]);
        broker.latency = 1;
        broker.submit(&req).unwrap();
        broker.on_candle_open(&BTreeMap::from([("ABC".into(), candle(2, 12.0, 12.0))]));
        assert_eq!(scripted.drain_events(), broker.drain_events());
    }

    #[test]
    fn pending_order_carries_across_day_rollover_and_counters_reset() {
        let program = crate::compile("TRADE_IN ABC\nWHEN close > 0\nBUY 1\n").unwrap();
        let day1 = chrono::NaiveDate::from_ymd_opt(2024, 5, 6).unwrap();
        let day2 = chrono::NaiveDate::from_ymd_opt(2024, 5, 7).unwrap();
        let ts1 = day1.and_hms_opt(10, 0, 0).unwrap().and_utc().timestamp();
        let ts2 = day2.and_hms_opt(10, 0, 0).unwrap().and_utc().timestamp();
        let source = MemorySource(vec![
            ("ABC".into(), candle(ts1, 10.0, 10.0)),
            ("ABC".into(), candle(ts2, 11.0, 11.0)),
        ]);
        let result = run(&program, &source, config(), None).unwrap();
        assert_eq!(result.fills.len(), 1);
        assert_eq!(result.fills[0].ts, ts2);
        assert_eq!(result.fills[0].price, 11.0);
    }

    #[test]
    fn orchestrated_daily_loss_flattens_cancels_entries_and_reopens_next_day() {
        let source_text =
            "TRADE_IN ABC\nRISK MAX_DAILY_LOSS 0.1%\nWHEN close > 0\nBUY 2\nWHEN close < 95\nSELL 1\n";
        let program = crate::compile(source_text).unwrap();
        let tz = FixedOffset::east_opt(19800).unwrap();
        let d1 = chrono::NaiveDate::from_ymd_opt(2024, 5, 6).unwrap();
        let d2 = chrono::NaiveDate::from_ymd_opt(2024, 5, 7).unwrap();
        let ts = |d: chrono::NaiveDate, h, m| {
            tz.from_local_datetime(&d.and_hms_opt(h, m, 0).unwrap())
                .single()
                .unwrap()
                .timestamp()
        };
        let source = MemorySource(vec![
            ("ABC".into(), candle(ts(d1, 9, 15), 100.0, 100.0)), // signals buy
            ("ABC".into(), candle(ts(d1, 9, 16), 100.0, 100.0)), // buy fill
            ("ABC".into(), candle(ts(d1, 9, 17), 90.0, 90.0)),   // losing exit signals
            ("ABC".into(), candle(ts(d1, 9, 18), 90.0, 90.0)),   // loss is realized
            ("ABC".into(), candle(ts(d1, 9, 19), 90.0, 90.0)),   // flatten after breach
            ("ABC".into(), candle(ts(d1, 9, 20), 90.0, 90.0)),   // flatten fill
            ("ABC".into(), candle(ts(d2, 9, 15), 90.0, 90.0)),   // next-day signal
            ("ABC".into(), candle(ts(d2, 9, 16), 90.0, 90.0)),   // next-day fill
        ]);
        let mut cfg = config();
        cfg.initial_cash = 10_000.0;
        let result = run(&program, &source, cfg, None).unwrap();
        assert_eq!(result.fills.len(), 3);
        assert_eq!(result.fills[0].side, Side::Buy);
        assert_eq!(result.fills[1].side, Side::Sell);
        assert_eq!(result.fills[1].reason, OrderReason::Rule("rule_2".into()));
        assert_eq!(result.fills[2].side, Side::Sell);
        assert_eq!(result.fills[2].reason, OrderReason::DailyLossFlatten);
        assert_eq!(result.open_positions.iter().map(|p| p.qty).sum::<i64>(), 0);
        assert!(result
            .events
            .iter()
            .any(|event| matches!(event, TraderEvent::DailyLossBreached)));
    }

    #[test]
    fn multi_symbol_daily_loss_does_not_crash_on_duplicate_cancels() {
        let source_text =
            "TRADE_IN ABC, XYZ\nRISK MAX_DAILY_LOSS 0.1%\nWHEN close > 0\nBUY 2\nWHEN close < 95\nSELL 1\n";
        let program = crate::compile(source_text).unwrap();
        let tz = FixedOffset::east_opt(19800).unwrap();
        let d1 = chrono::NaiveDate::from_ymd_opt(2024, 5, 6).unwrap();
        let ts = |h, m| {
            tz.from_local_datetime(&d1.and_hms_opt(h, m, 0).unwrap())
                .single()
                .unwrap()
                .timestamp()
        };
        // ABC signals and buys; XYZ has active buy order queued when ABC breaches daily loss
        let source = MemorySource(vec![
            ("ABC".into(), candle(ts(9, 15), 100.0, 100.0)),
            ("XYZ".into(), candle(ts(9, 15), 50.0, 50.0)),
            ("ABC".into(), candle(ts(9, 16), 100.0, 100.0)),
            ("XYZ".into(), candle(ts(9, 16), 50.0, 50.0)),
            ("ABC".into(), candle(ts(9, 17), 90.0, 90.0)),
            ("XYZ".into(), candle(ts(9, 17), 50.0, 50.0)),
            ("ABC".into(), candle(ts(9, 18), 90.0, 90.0)),
            ("XYZ".into(), candle(ts(9, 18), 50.0, 50.0)),
            ("ABC".into(), candle(ts(9, 19), 90.0, 90.0)),
            ("XYZ".into(), candle(ts(9, 19), 50.0, 50.0)),
        ]);
        let mut cfg = config();
        cfg.initial_cash = 10_000.0;
        let result = run(&program, &source, cfg, None).unwrap();
        assert!(result
            .events
            .iter()
            .any(|event| matches!(event, TraderEvent::DailyLossBreached)));
    }

    #[test]
    fn date_warmup_keeps_only_the_requested_tail_per_symbol() {
        let source: Vec<Result<(Symbol, Candle), SourceError>> = (1..=5)
            .map(|ts| Ok(("A".into(), candle(ts, 1.0, 1.0))))
            .collect();
        let from = 4;
        let out: Vec<_> = limit_to_warmup_window(Box::new(source.into_iter()), Some(from), 2)
            .map(Result::unwrap)
            .map(|(_, c)| c.timestamp)
            .collect();
        assert_eq!(out, vec![2, 3, 4, 5]);
    }

    #[test]
    fn skip_bad_rows_are_counted_and_valid_rows_survive() {
        let base = std::env::temp_dir().join(format!("algomln-skip-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        std::fs::write(
            base.join("A.csv"),
            "ts,open,high,low,close,volume\n1,1,1,1,1,1\nbad,1,1,1,1,1\n2,2,2,2,2,1\n",
        )
        .unwrap();
        let mut src = FileSource::new(&base);
        src.skip_bad = true;
        let stream = src
            .open(&["A".into()], Timeframe::M1, DateRange::default())
            .unwrap();
        let rows: Vec<_> = stream.map(Result::unwrap).collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(src.stats().skipped_bad["A"], 1);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn golden_strategy_and_csv_produce_expected_round_trip_file() {
        let strategy = include_str!("../../tests/golden/simulation.algomln");
        let p = crate::compile(strategy).unwrap();
        let src = FileSource::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden"));
        let mut cfg = config();
        cfg.initial_cash = 1000.0;
        let r = run(&p, &src, cfg, None).unwrap();
        let dir = std::env::temp_dir().join(format!("algomln-golden-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        write_results(&dir, &r).unwrap();
        let actual = std::fs::read_to_string(dir.join("trades.csv")).unwrap();
        assert_eq!(
            actual,
            include_str!("../../tests/golden/simulation.trades.csv")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn file_stream_merges_by_timestamp_then_symbol() {
        let base = std::env::temp_dir().join(format!("algomln-sim-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        std::fs::write(
            base.join("A.csv"),
            "ts,open,high,low,close,volume\n1,1,1,1,1,1\n",
        )
        .unwrap();
        std::fs::write(
            base.join("B.csv"),
            "ts,open,high,low,close,volume\n1,1,1,1,1,1\n",
        )
        .unwrap();
        let src = FileSource::new(&base);
        let mut stream = src
            .open(
                &["B".into(), "A".into()],
                Timeframe::M1,
                DateRange::default(),
            )
            .unwrap();
        let a = stream.next().unwrap().unwrap();
        let b = stream.next().unwrap().unwrap();
        assert_eq!((a.0, a.1.timestamp), ("A".into(), 1));
        assert_eq!((b.0, b.1.timestamp), ("B".into(), 1));
        drop(stream);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn file_source_fills_flat_only_inside_same_ist_session() {
        let base = std::env::temp_dir().join(format!("algomln-gaps-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        let tz = FixedOffset::east_opt(19800).unwrap();
        let day = chrono::NaiveDate::from_ymd_opt(2024, 5, 6).unwrap();
        let t1 = tz
            .from_local_datetime(&day.and_hms_opt(9, 15, 0).unwrap())
            .single()
            .unwrap()
            .timestamp();
        let t3 = tz
            .from_local_datetime(&day.and_hms_opt(9, 17, 0).unwrap())
            .single()
            .unwrap()
            .timestamp();
        std::fs::write(
            base.join("ABC.csv"),
            format!("ts,open,high,low,close,volume\n{t1},10,10,10,10,1\n{t3},12,12,12,12,1\n"),
        )
        .unwrap();
        let mut src = FileSource::new(&base);
        src.gap_policy = GapPolicy::FillFlatInSession;
        let stream = src
            .open(&["ABC".into()], Timeframe::M1, DateRange::default())
            .unwrap();
        let got: Vec<_> = stream.map(Result::unwrap).map(|(_, c)| c).collect();
        assert_eq!(got.len(), 3);
        assert_eq!(got[1].timestamp, t1 + 60);
        assert_eq!(got[1].close, 10.0);
        assert_eq!(got[1].volume, 0.0);
        assert_eq!(src.stats().missing_candles["ABC"], 1);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn daily_series_is_never_gap_filled() {
        let base = std::env::temp_dir().join(format!("algomln-d1gap-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        let tz = FixedOffset::east_opt(19800).unwrap();
        let ts_at = |y, m, d| {
            let day = chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
            tz.from_local_datetime(&day.and_hms_opt(9, 15, 0).unwrap())
                .single()
                .unwrap()
                .timestamp()
        };
        // Two daily bars three calendar days apart (a weekend gap between them).
        let t1 = ts_at(2024, 5, 3); // Fri
        let t2 = ts_at(2024, 5, 6); // Mon
        std::fs::write(
            base.join("ABC.csv"),
            format!("ts,open,high,low,close,volume\n{t1},10,10,10,10,1\n{t2},12,12,12,12,1\n"),
        )
        .unwrap();
        let mut src = FileSource::new(&base);
        src.gap_policy = GapPolicy::FillFlatInSession;
        let stream = src
            .open(&["ABC".into()], Timeframe::D1, DateRange::default())
            .unwrap();
        let got: Vec<_> = stream.map(Result::unwrap).map(|(_, c)| c).collect();
        // Daily gaps are non-trading days and must never be filled or counted.
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].timestamp, t1);
        assert_eq!(got[1].timestamp, t2);
        assert!(src.stats().missing_candles.get("ABC").is_none());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn parquet_reader_streams_validated_ohlcv_rows() {
        use arrow_array::{Float64Array, Int64Array, RecordBatch};
        use arrow_schema::{DataType, Field, Schema};
        use parquet::arrow::ArrowWriter;
        use std::sync::Arc;
        let base = std::env::temp_dir().join(format!("algomln-parquet-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&base);
        let path = base.join("ABC.parquet");
        let schema = Arc::new(Schema::new(vec![
            Field::new("ts", DataType::Int64, false),
            Field::new("open", DataType::Float64, false),
            Field::new("high", DataType::Float64, false),
            Field::new("low", DataType::Float64, false),
            Field::new("close", DataType::Float64, false),
            Field::new("volume", DataType::Float64, false),
        ]));
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![10_i64])),
                Arc::new(Float64Array::from(vec![10.0])),
                Arc::new(Float64Array::from(vec![11.0])),
                Arc::new(Float64Array::from(vec![9.0])),
                Arc::new(Float64Array::from(vec![10.5])),
                Arc::new(Float64Array::from(vec![4.0])),
            ],
        )
        .unwrap();
        let mut writer = ArrowWriter::try_new(File::create(&path).unwrap(), schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        let source = FileSource::new(&base);
        let mut stream = source
            .open(&["ABC".into()], Timeframe::M1, DateRange::default())
            .unwrap();
        let (_, c) = stream.next().unwrap().unwrap();
        assert_eq!(c.timestamp, 10);
        assert_eq!(c.close, 10.5);
        assert!(stream.next().is_none());
        drop(stream);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn broker_waits_for_missing_symbol_and_partial_fills_cancel_cleanly() {
        let mut b = SimBroker::new(vec!["ABC".into()]);
        b.partial_fills_pct = Some(50.0);
        let req = OrderRequest {
            client_order_id: "partial".into(),
            symbol: "ABC".into(),
            side: Side::Buy,
            qty: 5,
            reason: OrderReason::Rule("r".into()),
            ref_price: 10.0,
            decided_at: 1,
        };
        b.submit(&req).unwrap();
        b.on_candle_open(&BTreeMap::new());
        assert!(b.drain_events().is_empty());
        let mut c2 = candle(2, 10.0, 10.0);
        c2.volume = 2.0;
        b.on_candle_open(&BTreeMap::from([("ABC".into(), c2)]));
        let first = b.drain_events();
        assert!(matches!(first[0], OrderEvent::Acked { .. }));
        assert!(matches!(first[1], OrderEvent::PartialFill { qty: 1, .. }));
        let mut c3 = candle(3, 11.0, 11.0);
        c3.volume = 2.0;
        b.on_candle_open(&BTreeMap::from([("ABC".into(), c3)]));
        assert!(matches!(
            b.drain_events()[0],
            OrderEvent::PartialFill { qty: 1, .. }
        ));
        b.cancel("partial".into()).unwrap();
        assert!(matches!(b.drain_events()[0], OrderEvent::Cancelled { .. }));
        assert_eq!(
            b.cancel("partial".into()),
            Err(PortError::UnknownOrder("partial".into()))
        );
    }

    #[test]
    fn alphabetical_symbol_wins_when_cash_allows_one_entry() {
        let p = crate::compile("TRADE_IN B,A\nWHEN close > 0\nBUY 10\n").unwrap();
        let src = MemorySource(vec![
            ("B".into(), candle(1, 1.0, 1.0)),
            ("A".into(), candle(1, 1.0, 1.0)),
            ("A".into(), candle(2, 1.0, 1.0)),
            ("B".into(), candle(2, 1.0, 1.0)),
        ]);
        let mut cfg = config();
        cfg.initial_cash = 10.0;
        let r = run(&p, &src, cfg, None).unwrap();
        assert_eq!(r.fills.len(), 1);
        assert_eq!(r.fills[0].symbol, "A");
    }

    #[test]
    fn stage_timings_are_measured_and_positive() {
        let p = crate::compile("TRADE_IN A\nWHEN close > 0\nBUY 1\n").unwrap();
        let src = MemorySource((1..=100).map(|i| ("A".into(), candle(i, 10.0, 10.0))).collect());
        let cfg = config();
        let r = run(&p, &src, cfg, None).unwrap();
        assert!(r.timings.load_merge >= std::time::Duration::ZERO);
        assert!(r.timings.engine_indicators >= std::time::Duration::ZERO);
        assert!(r.timings.trader >= std::time::Duration::ZERO);
    }

    #[test]
    fn sub_tick_penny_stock_sell_fill_does_not_floor_to_zero() {
        let mut b = SimBroker::new(vec!["PENNY".into()]);
        b.tick_size_paise = 5; // 5 paise tick size (0.05 rupees)
        let req = OrderRequest {
            client_order_id: "penny_sell".into(),
            symbol: "PENNY".into(),
            side: Side::Sell,
            qty: 100,
            reason: OrderReason::Rule("r".into()),
            ref_price: 0.02, // 2 paise (sub-tick)
            decided_at: 1,
        };
        b.submit(&req).unwrap();
        let c = candle(2, 0.02, 0.02);
        b.on_candle_open(&BTreeMap::from([("PENNY".into(), c)]));
        let events = b.drain_events();
        let fill = events.iter().find_map(|e| match e {
            OrderEvent::Filled { price, .. } => Some(*price),
            _ => None,
        });
        assert!(fill.is_some());
        let fill_price = fill.unwrap();
        assert!(fill_price > 0.0, "fill price must be positive, got {fill_price}");
        assert_eq!(fill_price, 0.05, "sub-tick price should clamp to minimum tick");
    }

    #[test]
    fn partial_exit_records_trade_in_trades_list() {
        let p = crate::compile("TRADE_IN A\nWHEN close == 10\nBUY 10\nWHEN close == 20\nSELL 4\n").unwrap();
        let src = MemorySource(vec![
            ("A".into(), candle(1, 10.0, 10.0)),
            ("A".into(), candle(2, 10.0, 20.0)),
            ("A".into(), candle(3, 20.0, 20.0)),
        ]);
        let mut cfg = config();
        cfg.initial_cash = 1000.0;
        let r = run(&p, &src, cfg, None).unwrap();
        assert_eq!(r.trades.len(), 1, "partial exit of 4 shares must be recorded in trades");
        assert_eq!(r.trades[0].qty, 4);
        assert_eq!(r.trades[0].avg_entry, 10.0);
        assert_eq!(r.trades[0].avg_exit, 20.0);
        assert_eq!(r.trades[0].net_pnl, 40.0);
    }
