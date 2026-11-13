use super::*;
use dhan_api::Scrip;
use std::sync::atomic::{AtomicUsize, Ordering};

struct FakeHistory {
    calls: Arc<AtomicUsize>,
    empty_first: bool,
}

struct CountingHistory {
    calls: Arc<AtomicUsize>,
}

impl HistoryApi for CountingHistory {
    fn intraday(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: NaiveDate,
        _: NaiveDate,
    ) -> Result<Vec<Bar>, DataError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }

    fn daily(
        &self,
        _: &str,
        _: &str,
        _: NaiveDate,
        _: NaiveDate,
    ) -> Result<Vec<Bar>, DataError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }
}

impl HistoryApi for FakeHistory {
    fn intraday(
        &self,
        _: &str,
        _: &str,
        _: &str,
        from: NaiveDate,
        _: NaiveDate,
    ) -> Result<Vec<Bar>, DataError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.empty_first && call == 0 {
            return Ok(Vec::new());
        }
        let ts = from
            .and_hms_opt(9, 15, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        Ok(vec![bar(ts, 100.0)])
    }

    fn daily(
        &self,
        _: &str,
        _: &str,
        from: NaiveDate,
        _: NaiveDate,
    ) -> Result<Vec<Bar>, DataError> {
        self.intraday("", "", "", from, from)
    }
}

fn bar(ts: i64, close: f64) -> Bar {
    Bar {
        ts,
        open: close,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: 1.0,
    }
}

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "algomln-{name}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ))
}

#[test]
fn chunks_are_89_inclusive_and_daily_fallback_is_configurable() {
    let d = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let xs = chunk_dates(d, d + chrono::Days::new(179));
    assert_eq!(xs.len(), 3);
    assert_eq!((xs[0].1 - xs[0].0).num_days() + 1, 89);
    assert_eq!(
        chunk_dates_limited(d, d + chrono::Days::new(10), 5).len(),
        3
    );
}

#[test]
fn planner_uses_89_calendar_day_chunks_and_one_daily_request() {
    let root = temp("planner-count");
    let db = root.join("catalog.sqlite");
    let mut cat = Catalog::open(&db).unwrap();
    cat.refresh_instruments(
        &[Scrip {
            security_id: "1".into(),
            symbol: "ABC".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: false,
        }],
        0,
    )
    .unwrap();
    let from = NaiveDate::from_ymd_opt(2021, 10, 31).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
    assert_eq!(chunk_dates(from, to).len(), 21);
    assert_eq!(
        missing_date_ranges(&cat, "1", Tf::M1, from, to).unwrap(),
        vec![(from + chrono::Days::new(1), to - chrono::Days::new(1))]
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let mut update = Updater {
        client: Box::new(CountingHistory {
            calls: calls.clone(),
        }),
        catalog: Catalog::open(&db).unwrap(),
        store: Store::new(&root),
        max_retries: 0,
        margin_seconds: 30,
        reference_symbol_id: None,
        budget_cap: DEFAULT_BUDGET_CAP,
    };
    update.update("1", Tf::M1, "EQUITY", from, to).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 21);
    let calls_d = Arc::new(AtomicUsize::new(0));
    let mut daily = Updater {
        client: Box::new(CountingHistory {
            calls: calls_d.clone(),
        }),
        catalog: Catalog::open(&db).unwrap(),
        store: Store::new(&root),
        max_retries: 0,
        margin_seconds: 30,
        reference_symbol_id: None,
        budget_cap: DEFAULT_BUDGET_CAP,
    };
    daily.update("1", Tf::D1, "EQUITY", from, to).unwrap();
    assert_eq!(calls_d.load(Ordering::SeqCst), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn planner_merges_middle_gaps_over_covered_week_and_weekend() {
    let root = temp("planner-gaps");
    let mut cat = Catalog::open(root.join("catalog.sqlite")).unwrap();
    cat.refresh_instruments(
        &[Scrip {
            security_id: "1".into(),
            symbol: "ABC".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: false,
        }],
        0,
    )
    .unwrap();
    let from = NaiveDate::from_ymd_opt(2026, 10, 19).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 10, 30).unwrap();
    for n in 0..=(to - from).num_days() {
        let day = from + chrono::Days::new(n as u64);
        if day.weekday().number_from_monday() <= 5 && day != from && day != to {
            cat.mark_session_empty("1", Tf::M1, day).unwrap();
        }
    }
    assert_eq!(
        missing_date_ranges(&cat, "1", Tf::M1, from, to).unwrap(),
        vec![(from, to)]
    );
    let calls = Arc::new(AtomicUsize::new(0));
    let mut updater = Updater {
        client: Box::new(CountingHistory {
            calls: calls.clone(),
        }),
        catalog: Catalog::open(root.join("catalog.sqlite")).unwrap(),
        store: Store::new(&root),
        max_retries: 0,
        margin_seconds: 0,
        reference_symbol_id: None,
        budget_cap: DEFAULT_BUDGET_CAP,
    };
    updater.update("1", Tf::M1, "EQUITY", from, to).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let middle = from + chrono::Days::new(1);
    let gap_from = from;
    let gap_to = to;
    let mut second = Catalog::open(root.join("other.sqlite")).unwrap();
    second
        .refresh_instruments(
            &[Scrip {
                security_id: "2".into(),
                symbol: "DEF".into(),
                lot_size: 1,
                tick_size_paise: 5,
                instrument: "EQUITY".into(),
                is_fno: false,
            }],
            0,
        )
        .unwrap();
    for n in 0..=(gap_to - gap_from).num_days() {
        let day = gap_from + chrono::Days::new(n as u64);
        if day.weekday().number_from_monday() <= 5 && day != middle {
            second.mark_session_empty("2", Tf::M1, day).unwrap();
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mut updater = Updater {
        client: Box::new(CountingHistory {
            calls: calls.clone(),
        }),
        catalog: second,
        store: Store::new(root.join("other-store")),
        max_retries: 0,
        margin_seconds: 0,
        reference_symbol_id: None,
        budget_cap: DEFAULT_BUDGET_CAP,
    };
    updater
        .update("2", Tf::M1, "EQUITY", gap_from, gap_to)
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn overlap_classifies_split_ratio_and_revision() {
    let a = vec![bar(60, 100.0), bar(120, 200.0)];
    let split = vec![bar(60, 50.0), bar(120, 100.0)];
    assert_eq!(check_overlap(&a, &split).0, "corporate_action");
    let near = vec![bar(60, 99.0), bar(120, 198.0)];
    assert_eq!(check_overlap(&a, &near).0, "corporate_action");
    let revision = vec![bar(60, 101.0), bar(120, 200.0)];
    assert_eq!(check_overlap(&a, &revision).0, "revised");
    assert_eq!(check_overlap(&a, &a).0, "identical");
}

#[test]
fn updater_is_idempotent_and_partial_retry_resumes_coverage() {
    let root = temp("updater");
    let db = root.join("catalog.sqlite");
    let from = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
    let to = from + chrono::Days::new(1);
    let fake = Arc::new(FakeHistory {
        calls: Arc::new(AtomicUsize::new(0)),
        empty_first: false,
    });
    let mut cat = Catalog::open(&db).unwrap();
    cat.refresh_instruments(
        &[Scrip {
            security_id: "7".into(),
            symbol: "ABC".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: false,
        }],
        0,
    )
    .unwrap();
    drop(cat);
    let mut updater = Updater {
        client: Box::new(FakeHistory {
            calls: fake.calls.clone(),
            empty_first: fake.empty_first,
        }),
        catalog: Catalog::open(&db).unwrap(),
        store: Store::new(&root),
        max_retries: 0,
        margin_seconds: 0,
        reference_symbol_id: Some("7".into()),
        budget_cap: DEFAULT_BUDGET_CAP,
    };
    assert!(updater.update("7", Tf::M1, "EQUITY", from, to).unwrap());
    let after_first = fake.calls.load(Ordering::SeqCst);
    assert!(after_first >= 1);
    let covered = updater
        .catalog
        .covered_dates("7", Tf::M1, from, to)
        .unwrap();
    assert_eq!(covered.len(), 2);
    let status = updater.catalog.status_rows().unwrap();
    assert_eq!(status.len(), 1);
    assert_eq!(status[0].symbol, "ABC");
    assert_eq!(status[0].bars, 1);
    assert_eq!(status[0].coverage.len(), 1);
    assert!(!updater.update("7", Tf::M1, "EQUITY", from, to).unwrap());
    assert_eq!(fake.calls.load(Ordering::SeqCst), after_first);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn confirmed_empty_calendar_dates_are_idempotent_and_cover_sessions() {
    let root = temp("calendar");
    let mut c = Catalog::open(root.join("catalog.sqlite")).unwrap();
    c.refresh_instruments(
        &[Scrip {
            security_id: "1".into(),
            symbol: "ABC".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: false,
        }],
        0,
    )
    .unwrap();
    let day = NaiveDate::from_ymd_opt(2026, 1, 26).unwrap();
    c.mark_session_empty("1", Tf::M1, day).unwrap();
    c.mark_session_empty("1", Tf::M1, day).unwrap();
    assert_eq!(c.covered_dates("1", Tf::M1, day, day).unwrap(), vec![day]);
    assert!(c.missing_dates("1", Tf::M1, day, day).unwrap().is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn historical_daily_bars_are_not_filtered_by_current_day_cutoff() {
    let root = temp("daily-history");
    let db = root.join("catalog.sqlite");
    let d = NaiveDate::from_ymd_opt(2020, 1, 2).unwrap();
    let ts = d
        .and_hms_opt(9, 15, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp();
    let mut cat = Catalog::open(&db).unwrap();
    cat.refresh_instruments(
        &[Scrip {
            security_id: "5".into(),
            symbol: "DAILY".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: false,
        }],
        0,
    )
    .unwrap();
    drop(cat);
    struct DailyFake(i64);
    impl HistoryApi for DailyFake {
        fn intraday(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: NaiveDate,
            _: NaiveDate,
        ) -> Result<Vec<Bar>, DataError> {
            Ok(Vec::new())
        }
        fn daily(
            &self,
            _: &str,
            _: &str,
            _: NaiveDate,
            _: NaiveDate,
        ) -> Result<Vec<Bar>, DataError> {
            Ok(vec![bar(self.0, 100.)])
        }
    }
    let mut updater = Updater {
        client: Box::new(DailyFake(ts)),
        catalog: Catalog::open(&db).unwrap(),
        store: Store::new(&root),
        max_retries: 0,
        margin_seconds: 0,
        reference_symbol_id: None,
        budget_cap: DEFAULT_BUDGET_CAP,
    };
    updater.update("5", Tf::D1, "EQUITY", d, d).unwrap();
    assert_eq!(
        updater.catalog.covered_dates("5", Tf::D1, d, d).unwrap(),
        vec![d]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn progress_filter_handles_normal_and_short_sessions() {
    let d = NaiveDate::from_ymd_opt(2026, 10, 20).unwrap();
    let open = d
        .and_hms_opt(9, 15, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp();
    let end = d
        .and_hms_opt(15, 30, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp();
    let normal = (0..375).map(|n| bar(open + n * 60, 1.0)).collect();
    assert_eq!(
        complete_intraday(normal, end, Tf::M1, &HashMap::new(), 0).len(),
        375
    );
    let short = d
        .and_hms_opt(15, 15, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp();
    let ends = HashMap::from([(d.to_string(), short)]);
    let short_rows = (0..360).map(|n| bar(open + n * 60, 1.0)).collect();
    assert_eq!(
        complete_intraday(short_rows, short, Tf::M1, &ends, 0).len(),
        360
    );
}

#[test]
fn atomic_store_and_catalog_reconcile() {
    let root = temp("atomic");
    let db = root.join("catalog.sqlite");
    let store = Store::new(&root);
    let d1 = NaiveDate::from_ymd_opt(2026, 1, 30).unwrap();
    let ts1 = d1
        .and_hms_opt(15, 20, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp();
    let d2 = NaiveDate::from_ymd_opt(2026, 2, 2).unwrap();
    let ts2 = d2
        .and_hms_opt(9, 15, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp();
    let parts = store
        .write_partitions("1", Tf::M1, &[bar(ts1, 10.0), bar(ts2, 11.0)])
        .unwrap();
    assert_eq!(parts.len(), 2);
    {
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(
            &[Scrip {
                security_id: "1".into(),
                symbol: "ABC".into(),
                lot_size: 1,
                tick_size_paise: 5,
                instrument: "EQUITY".into(),
                is_fno: false,
            }],
            0,
        )
        .unwrap();
        let sid = cat.series_for_import("1", Tf::M1).unwrap();
        for part in &parts {
            cat.record_partition(sid, &store, part).unwrap();
        }
        cat.record_session(sid, d1, ts1, ts1 + 60, 1).unwrap();
        cat.record_session(sid, d2, ts2, ts2 + 60, 1).unwrap();
        assert!(cat.has_coverage("1", Tf::M1, ts1, ts1).unwrap());
        store.reconcile(&mut cat).unwrap();
        std::fs::remove_file(&parts[0].path).unwrap();
        store.reconcile(&mut cat).unwrap();
        assert!(!cat.has_coverage("1", Tf::M1, ts1, ts1).unwrap());
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn orphan_partition_is_reported_after_write_before_catalog_commit() {
    let root = temp("orphan");
    let db = root.join("catalog.sqlite");
    let store = Store::new(&root);
    let ts = NaiveDate::from_ymd_opt(2026, 4, 1)
        .unwrap()
        .and_hms_opt(9, 15, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp();
    let parts = store
        .write_partitions("99", Tf::M1, &[bar(ts, 42.0)])
        .unwrap();
    let mut cat = Catalog::open(db).unwrap();
    store.reconcile(&mut cat).unwrap();
    store.reconcile(&mut cat).unwrap();
    assert_eq!(cat.quality("reconcile_orphan_file").unwrap(), 1);
    assert!(parts[0].path.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn coverage_details_joins_series_to_partition_rows() {
    let root = temp("coverage-details");
    let db = root.join("catalog.sqlite");
    let store = Store::new(&root);
    let ts = NaiveDate::from_ymd_opt(2026, 10, 28)
        .unwrap()
        .and_hms_opt(9, 15, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp();
    let parts = store
        .write_partitions("2885", Tf::M1, &[bar(ts, 100.0), bar(ts + 60, 101.0)])
        .unwrap();
    let mut cat = Catalog::open(db).unwrap();
    cat.refresh_instruments(
        &[Scrip {
            security_id: "2885".into(),
            symbol: "RELIANCE".into(),
            lot_size: 1,
            tick_size_paise: 10,
            instrument: "EQUITY".into(),
            is_fno: false,
        }],
        0,
    )
    .unwrap();
    let sid = cat.series_for_import("2885", Tf::M1).unwrap();
    for p in &parts {
        cat.record_partition(sid, &store, p).unwrap();
    }
    assert_eq!(
        cat.coverage_details().unwrap(),
        vec![("RELIANCE".into(), "1m".into(), 2)]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn open_migrates_legacy_catalog_without_verify_and_sets_user_version() {
    let root = temp("migration");
    fs::create_dir_all(&root).unwrap();
    let db = root.join("catalog.sqlite");
    let legacy = Connection::open(&db).unwrap();
    legacy.execute_batch("CREATE TABLE instruments(security_id TEXT PRIMARY KEY,symbol TEXT NOT NULL UNIQUE,segment TEXT NOT NULL,lot_size INTEGER NOT NULL,tick_size_paise INTEGER NOT NULL,refreshed_at INTEGER NOT NULL); INSERT INTO instruments VALUES('1','ABC','NSE_EQ',1,5,0);").unwrap();
    drop(legacy);
    let cat = Catalog::open(&db).unwrap();
    assert_eq!(cat.instrument("1").unwrap(), "EQUITY");
    let version: i64 = cat
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2);
    assert!(!cat.is_fno("1").unwrap());
    assert_eq!(cat.status_rows().unwrap().len(), 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn catalog_migration_from_v1_populates_is_fno_on_refresh() {
    let root = temp("migration-v1-to-v2");
    fs::create_dir_all(&root).unwrap();
    let db = root.join("catalog.sqlite");
    let legacy = Connection::open(&db).unwrap();
    legacy.execute_batch("PRAGMA user_version=1; CREATE TABLE instruments(security_id TEXT PRIMARY KEY,symbol TEXT NOT NULL UNIQUE,segment TEXT NOT NULL,lot_size INTEGER NOT NULL,tick_size_paise INTEGER NOT NULL,instrument TEXT NOT NULL DEFAULT 'EQUITY',refreshed_at INTEGER NOT NULL); INSERT INTO instruments VALUES('2885','RELIANCE','NSE_EQ',1,10,'EQUITY',0);").unwrap();
    drop(legacy);
    let mut cat = Catalog::open(&db).unwrap();
    let version: i64 = cat
        .conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 2);
    assert!(!cat.is_fno("2885").unwrap());
    cat.refresh_instruments(
        &[Scrip {
            security_id: "2885".into(),
            symbol: "RELIANCE".into(),
            lot_size: 1,
            tick_size_paise: 10,
            instrument: "EQUITY".into(),
            is_fno: true,
        }],
        Utc::now().timestamp(),
    )
    .unwrap();
    assert!(cat.is_fno("2885").unwrap());
    let (total, fno) = cat.instrument_counts().unwrap();
    assert_eq!(total, 1);
    assert_eq!(fno, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn status_ranges_coalesce_touching_ist_dates_per_series() {
    let root = temp("status-ranges");
    let mut cat = Catalog::open(root.join("catalog.sqlite")).unwrap();
    cat.refresh_instruments(
        &[Scrip {
            security_id: "1".into(),
            symbol: "ABC".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: false,
        }],
        0,
    )
    .unwrap();
    let a = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
    let dates = [
        a,
        a + chrono::Days::new(1),
        a + chrono::Days::new(2),
        a + chrono::Days::new(3),
        a + chrono::Days::new(4),
        a + chrono::Days::new(7),
        a + chrono::Days::new(9),
        a + chrono::Days::new(11),
    ];
    for d in dates {
        cat.record_coverage(
            "1",
            Tf::M1,
            local_midnight(d),
            local_midnight(d + chrono::Days::new(1)) - 1,
            "data",
        )
        .unwrap();
    }
    cat.record_calendar(a + chrono::Days::new(8), false, "holiday")
        .unwrap();
    let rows = cat.status_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].coverage,
        vec![
            CoverageRange {
                from: a,
                to: a + chrono::Days::new(9),
                kind: "data".into()
            },
            CoverageRange {
                from: a + chrono::Days::new(11),
                to: a + chrono::Days::new(11),
                kind: "data".into()
            }
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn session_regime_change_detects_august_2026_once() {
    let mut shapes = BTreeMap::new();
    for n in 0..5 {
        let d = NaiveDate::from_ymd_opt(2026, 7, 27).unwrap() + chrono::Days::new(n);
        let end = d
            .and_hms_opt(15, 29, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        shapes.insert(d, (0, end, 375));
    }
    let start = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
    for n in 0..12 {
        let d = start + chrono::Days::new(n);
        let end = d
            .and_hms_opt(15, 14, 0)
            .unwrap()
            .and_local_timezone(Kolkata)
            .single()
            .unwrap()
            .timestamp();
        shapes.insert(d, (0, end, 360));
    }
    let changes = session_regime_changes(&shapes);
    assert_eq!(changes, vec![start + chrono::Days::new(4)]);
}

#[test]
fn quality_rows_joins_symbol_tf_and_formats_ist_date() {
    let root = temp("quality_rows");
    let db = root.join("catalog.sqlite");
    let mut cat = Catalog::open(&db).unwrap();
    cat.refresh_instruments(
        &[Scrip {
            security_id: "101".into(),
            symbol: "RELIANCE".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: false,
        }],
        0,
    )
    .unwrap();
    cat.series_for_import("101", Tf::M1).unwrap();
    let day = NaiveDate::from_ymd_opt(2026, 8, 7).unwrap();
    cat.mark_quality(
        "101",
        Tf::M1,
        &Quality {
            kind: "session_regime_change".into(),
            ts: Some(local_midnight(day)),
            detail: "derived session end mode changed".into(),
        },
    )
    .unwrap();
    let rows = cat.quality_rows().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0],
        QualityRow {
            symbol: "RELIANCE".into(),
            tf: "1m".into(),
            date: "2026-08-07".into(),
            kind: "session_regime_change".into(),
        }
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn repeated_identical_write_keeps_checksum_and_bytes() {
    let root = temp("idempotent");
    let store = Store::new(&root);
    let ts = NaiveDate::from_ymd_opt(2026, 2, 2)
        .unwrap()
        .and_hms_opt(9, 15, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp();
    let b = bar(ts, 10.0);
    let a = store
        .write_partitions("x", Tf::M1, std::slice::from_ref(&b))
        .unwrap();
    let bytes = fs::read(&a[0].path).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(10));
    let z = store.write_partitions("x", Tf::M1, &[b]).unwrap();
    assert_eq!(a[0].checksum, z[0].checksum);
    assert_eq!(bytes, fs::read(&z[0].path).unwrap());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn month_partition_uses_ist_day() {
    let root = temp("ist");
    let store = Store::new(&root);
    let local = NaiveDate::from_ymd_opt(2026, 1, 31)
        .unwrap()
        .and_hms_opt(23, 59, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp();
    let p = store
        .write_partitions("1", Tf::M1, &[bar(local, 10.0)])
        .unwrap();
    assert!(p[0].path.to_string_lossy().contains("2026-01"));
    let _ = fs::remove_dir_all(root);
}

fn scrip(id: &str, sym: &str) -> Scrip {
    Scrip {
        security_id: id.into(),
        symbol: sym.into(),
        lot_size: 1,
        tick_size_paise: 5,
        instrument: "EQUITY".into(),
        is_fno: false,
    }
}

#[test]
fn plan_requests_counts_89_day_chunks_and_single_daily() {
    let root = temp("plan-count");
    let mut cat = Catalog::open(root.join("catalog.sqlite")).unwrap();
    cat.refresh_instruments(&[scrip("1", "ABC")], 0).unwrap();
    let from = NaiveDate::from_ymd_opt(2021, 10, 31).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
    assert_eq!(plan_requests(&cat, "1", Tf::M1, from, to).unwrap(), 21);
    assert_eq!(plan_requests(&cat, "1", Tf::D1, from, to).unwrap(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn dry_run_reports_per_tf_plan_and_sends_zero_requests() {
    let root = temp("bulk-dry");
    let db = root.join("catalog.sqlite");
    let mut cat = Catalog::open(&db).unwrap();
    cat.refresh_instruments(&[scrip("10", "AAA"), scrip("2", "BBB")], 0)
        .unwrap();
    drop(cat);
    let from = NaiveDate::from_ymd_opt(2021, 10, 31).unwrap();
    let to = NaiveDate::from_ymd_opt(2026, 10, 31).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bulk = BulkDownloader {
        make_client: move || {
            Box::new(CountingHistory { calls: c.clone() }) as Box<dyn HistoryApi>
        },
        catalog_path: db.clone(),
        store_root: root.clone(),
        config: BulkConfig::new(from, to, vec![Tf::D1, Tf::M1]),
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let report = bulk.dry_run().unwrap();
    assert_eq!(report.symbols_total, 2);
    assert_eq!(
        report.planned,
        vec![("1d".to_string(), 2), ("1m".to_string(), 42)]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bulk_orders_priority_symbols_first_then_security_id() {
    let root = temp("bulk-order");
    let db = root.join("catalog.sqlite");
    let mut cat = Catalog::open(&db).unwrap();
    cat.refresh_instruments(
        &[scrip("10", "AAA"), scrip("2", "BBB"), scrip("30", "CCC")],
        0,
    )
    .unwrap();
    drop(cat);
    let from = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
    let to = from + chrono::Days::new(1);
    let mut cfg = BulkConfig::new(from, to, vec![Tf::M1]);
    cfg.priority_symbols = vec!["CCC".into()];
    let bulk = BulkDownloader {
        make_client: || {
            Box::new(CountingHistory {
                calls: Arc::new(AtomicUsize::new(0)),
            }) as Box<dyn HistoryApi>
        },
        catalog_path: db.clone(),
        store_root: root.clone(),
        config: cfg,
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let cat = Catalog::open(&db).unwrap();
    let order = bulk.order(&cat).unwrap();
    let symbols: Vec<_> = order.iter().map(|(_, s, _)| s.clone()).collect();
    // CCC (priority) first, then by numeric security id: 2=BBB, 10=AAA.
    assert_eq!(symbols, vec!["CCC", "BBB", "AAA"]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bulk_stops_at_soft_cap_and_budget_survives_restart() {
    let root = temp("bulk-budget");
    let db = root.join("catalog.sqlite");
    let mut cat = Catalog::open(&db).unwrap();
    cat.refresh_instruments(&[scrip("1", "AAA"), scrip("2", "BBB")], 0)
        .unwrap();
    // Pre-seed today's budget so the cap is reached after the first symbol.
    let today = chrono::Utc::now().date_naive().to_string();
    cat.budget_add(&today, 1).unwrap();
    drop(cat);
    let from = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
    let to = from + chrono::Days::new(1);
    let mut cfg = BulkConfig::new(from, to, vec![Tf::D1]);
    cfg.budget_cap = 2; // room for exactly one more request
    let calls = Arc::new(AtomicUsize::new(0));
    let c = calls.clone();
    let bulk = BulkDownloader {
        make_client: move || {
            Box::new(FakeHistory {
                calls: c.clone(),
                empty_first: false,
            }) as Box<dyn HistoryApi>
        },
        catalog_path: db.clone(),
        store_root: root.clone(),
        config: cfg,
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let report = bulk.run(|_| {}).unwrap();
    assert!(report.budget_reached);
    // One request got through (budget 1 -> 2 == cap), the second symbol is skipped.
    let cat = Catalog::open(&db).unwrap();
    assert_eq!(cat.budget_used(&today).unwrap(), 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn bulk_resume_does_not_re_request_completed_ranges() {
    let root = temp("bulk-resume");
    let db = root.join("catalog.sqlite");
    let mut cat = Catalog::open(&db).unwrap();
    cat.refresh_instruments(&[scrip("7", "AAA")], 0).unwrap();
    drop(cat);
    let from = NaiveDate::from_ymd_opt(2026, 10, 26).unwrap();
    let to = from + chrono::Days::new(1);
    let calls = Arc::new(AtomicUsize::new(0));
    let make = {
        let calls = calls.clone();
        move || {
            Box::new(FakeHistory {
                calls: calls.clone(),
                empty_first: false,
            }) as Box<dyn HistoryApi>
        }
    };
    let mut cfg = BulkConfig::new(from, to, vec![Tf::M1]);
    cfg.reference_symbol_id = Some("7".into());
    let bulk = BulkDownloader {
        make_client: make,
        catalog_path: db.clone(),
        store_root: root.clone(),
        config: cfg,
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let first = bulk.run(|_| {}).unwrap();
    assert_eq!(first.symbols_completed, 1);
    let after_first = calls.load(Ordering::SeqCst);
    assert!(after_first >= 1);
    // Rerun: planner derives work from coverage, so nothing new is requested.
    let second = bulk.run(|_| {}).unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), after_first);
    assert_eq!(second.requests_used, 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn progress_line_matches_spec_shape() {
    let p = Progress {
        tf: "1m",
        done: 842,
        total: 2365,
        requests: 17_904,
        budget_cap: 90_000,
        budget_used: 17_904,
        rate: 3.9,
        failures: 3,
    };
    let line = p.line();
    assert!(line.starts_with("1m  842/2365 symbols | requests 17,904/90,000 today |"));
    assert!(line.contains("3.9 req/s"));
    assert!(line.ends_with("failures 3"));
}

#[test]
fn thousands_grouping_and_eta_formatting() {
    assert_eq!(group_thousands(90_000), "90,000");
    assert_eq!(group_thousands(17_904), "17,904");
    assert_eq!(group_thousands(5), "5");
    assert_eq!(format_eta(2 * 3600 + 41 * 60), "2h 41m");
    assert_eq!(format_eta(600), "10m");
    assert_eq!(format_eta(0), "0m");
}

#[test]
fn last_complete_bar_daily_cutoff_regression() {
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;
    let d = NaiveDate::from_ymd_opt(2024, 6, 10).unwrap();
    // 15:30 IST (before 16:00 cutoff)
    let before_cutoff = Kolkata
        .from_local_datetime(&d.and_hms_opt(15, 30, 0).unwrap())
        .single()
        .unwrap()
        .timestamp();
    let res_before = last_complete_bar(before_cutoff, Tf::D1, 0, 0);
    let expected_before = Kolkata
        .from_local_datetime(&NaiveDate::from_ymd_opt(2024, 6, 9).unwrap().and_hms_opt(9, 15, 0).unwrap())
        .single()
        .unwrap()
        .timestamp();
    assert_eq!(res_before, expected_before, "before 16:00 cutoff should return yesterday 09:15 IST");

    // 16:05 IST (after 16:00 cutoff)
    let after_cutoff = Kolkata
        .from_local_datetime(&d.and_hms_opt(16, 5, 0).unwrap())
        .single()
        .unwrap()
        .timestamp();
    let res_after = last_complete_bar(after_cutoff, Tf::D1, 0, 0);
    let expected_after = Kolkata
        .from_local_datetime(&d.and_hms_opt(9, 15, 0).unwrap())
        .single()
        .unwrap()
        .timestamp();
    assert_eq!(res_after, expected_after, "after 16:00 cutoff should return today 09:15 IST");
}

#[test]
fn bulk_concurrent_does_not_generate_false_orphans() {
    let root = temp("bulk-concurrent");
    let db = root.join("catalog.sqlite");
    let mut cat = Catalog::open(&db).unwrap();
    cat.refresh_instruments(&[scrip("1", "AAA"), scrip("2", "BBB")], 0).unwrap();
    drop(cat);

    let calls = Arc::new(AtomicUsize::new(0));
    let mut cfg = BulkConfig::new(
        NaiveDate::from_ymd_opt(2024, 6, 3).unwrap(),
        NaiveDate::from_ymd_opt(2024, 6, 3).unwrap(),
        vec![Tf::M1],
    );
    cfg.concurrency = 2;
    let bulk = BulkDownloader {
        make_client: {
            let calls = calls.clone();
            move || Box::new(CountingHistory { calls: calls.clone() })
        },
        catalog_path: db.clone(),
        store_root: root.clone(),
        config: cfg,
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let report = bulk.run(|_| {}).unwrap();
    assert_eq!(report.symbols_completed, 2);

    let cat = Catalog::open(&db).unwrap();
    let orphans: usize = cat
        .conn
        .query_row(
            "SELECT COUNT(*) FROM quality WHERE kind='reconcile_orphan_file'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(orphans, 0, "concurrent downloads should not flag sibling files as orphans");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn atomic_replace_replaces_existing_file_correctly() {
    let root = temp("atomic-replace");
    fs::create_dir_all(&root).unwrap();
    let target = root.join("target.txt");
    let source = root.join("source.txt");
    fs::write(&target, "old content").unwrap();
    fs::write(&source, "new content").unwrap();

    crate::store::atomic_replace(&source, &target).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), "new content");
    assert!(!source.exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn kway_merge_deterministic_symbol_tie_break() {
    use algomln::sim::CandleSource;
    use chrono::TimeZone;
    use chrono_tz::Asia::Kolkata;
    let root = temp("kway-merge-test");
    fs::create_dir_all(&root).unwrap();
    let db = root.join("catalog.sqlite");
    let mut cat = Catalog::open(&db).unwrap();
    cat.refresh_instruments(&[scrip("1", "AAPL"), scrip("2", "MSFT")], 0).unwrap();
    let store = Store::new(&root);
    let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let ts = Kolkata.from_local_datetime(&mon.and_hms_opt(9, 15, 0).unwrap()).single().unwrap().timestamp();
    let bar_a = Bar { ts, open: 10.0, high: 11.0, low: 9.0, close: 10.5, volume: 100.0 };
    let bar_m = Bar { ts, open: 20.0, high: 21.0, low: 19.0, close: 20.5, volume: 200.0 };
    let parts_a = store.write_partitions("1", Tf::M1, &[bar_a]).unwrap();
    let parts_m = store.write_partitions("2", Tf::M1, &[bar_m]).unwrap();
    let sid_a = cat.series_for_import("1", Tf::M1).unwrap();
    let sid_m = cat.series_for_import("2", Tf::M1).unwrap();
    for p in &parts_a { cat.record_partition(sid_a, &store, p).unwrap(); }
    for p in &parts_m { cat.record_partition(sid_m, &store, p).unwrap(); }
    cat.record_session(sid_a, mon, ts, ts, 1).unwrap();
    cat.record_session(sid_m, mon, ts, ts, 1).unwrap();
    cat.record_coverage("1", Tf::M1, ts - 60, ts + 3600, "data").unwrap();
    cat.record_coverage("2", Tf::M1, ts - 60, ts + 3600, "data").unwrap();
    drop(cat);

    let source = CatalogSource::new(Store::new(&root), &db);
    let syms = vec!["MSFT".to_string(), "AAPL".to_string()]; // unsorted input
    let range = algomln::sim::DateRange { from: Some(ts), to: Some(ts) };
    let stream = source.open(&syms, algomln::sim::Timeframe::M1, range).unwrap();
    let results: Vec<_> = stream.map(|r| r.unwrap()).collect();
    assert_eq!(results.len(), 2);
    // Even though both have identical timestamp, AAPL must come before MSFT due to symbol tie-break
    assert_eq!(results[0].0, "AAPL");
    assert_eq!(results[1].0, "MSFT");
    let _ = fs::remove_dir_all(root);
}
