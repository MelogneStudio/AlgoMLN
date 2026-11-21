use super::*;
use algomln::sim::CandleSource;
use crate::{Bar, Catalog, CatalogSource, Store, Tf};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread;

#[test]
fn all_22_indices_defined_and_unique() {
    let all = IndexAlias::all();
    assert_eq!(all.len(), 22);

    let mut aliases = HashSet::new();
    let mut display_names = HashSet::new();
    let mut dsl_keywords = HashSet::new();
    let mut csv_filenames = HashSet::new();

    for idx in all {
        assert!(aliases.insert(idx.alias()), "duplicate alias {}", idx.alias());
        assert!(
            display_names.insert(idx.display_name()),
            "duplicate display name {}",
            idx.display_name()
        );
        assert!(
            dsl_keywords.insert(idx.dsl_keyword()),
            "duplicate keyword {}",
            idx.dsl_keyword()
        );
        assert!(
            csv_filenames.insert(idx.csv_filename()),
            "duplicate csv filename {}",
            idx.csv_filename()
        );
    }
}

#[test]
fn bundled_seeds_parse_and_have_symbols() {
    let registry = IndexRegistry::load_with_cache_dir(None);
    let meta = registry.list_metadata();
    assert_eq!(meta.len(), 22);

    for idx in IndexAlias::all() {
        let symbols = registry.get_symbols(*idx);
        assert!(
            !symbols.is_empty(),
            "bundled seed for {} must have symbols",
            idx.display_name()
        );
        let entry = registry.get_entry(*idx).expect("entry must exist");
        assert_eq!(entry.symbols.len(), symbols.len());
        assert!(!entry.as_of.is_empty());
    }
}

#[test]
fn case_insensitive_keyword_matching() {
    for idx in IndexAlias::all() {
        let kw = idx.dsl_keyword();
        assert_eq!(IndexAlias::from_keyword(kw), Some(*idx));
        assert_eq!(IndexAlias::from_keyword(&kw.to_lowercase()), Some(*idx));
        // Without underscores
        let no_underscore = kw.replace('_', "");
        assert_eq!(
            IndexAlias::from_keyword(&no_underscore),
            Some(*idx),
            "failed without underscore: {no_underscore}"
        );
        assert_eq!(
            IndexAlias::from_keyword(&no_underscore.to_lowercase()),
            Some(*idx),
            "failed lowercase without underscore: {no_underscore}"
        );
    }

    assert_eq!(IndexAlias::from_keyword("niftybank"), Some(IndexAlias::NiftyBank));
    assert_eq!(IndexAlias::from_keyword("NIFTY BANK"), Some(IndexAlias::NiftyBank));
    assert_eq!(IndexAlias::from_keyword("nifty-50"), Some(IndexAlias::Nifty50));
    assert_eq!(
        IndexAlias::from_keyword("NIFTY_FINANCIAL_SERVICES"),
        Some(IndexAlias::NiftyFinancialServices)
    );
    assert_eq!(
        IndexAlias::from_keyword("niftyfinservice"),
        Some(IndexAlias::NiftyFinancialServices)
    );
    assert_eq!(IndexAlias::from_keyword("UNKNOWN_INDEX"), None);
}

#[test]
fn cache_overrides_bundled_seed() {
    let temp_dir = tempfile_dir("cache_override");
    let custom_file = IndexCacheFile {
        alias: "NIFTY_50".to_string(),
        as_of: "2099-01-01".to_string(),
        source_url: Some("https://example.com/ind_nifty50list.csv".to_string()),
        symbols: vec!["ABC".to_string(), "XYZ".to_string()],
    };
    write_cache_atomic(&temp_dir, IndexAlias::Nifty50, &custom_file).unwrap();

    let registry = IndexRegistry::load_with_cache_dir(Some(temp_dir.clone()));
    let syms = registry.get_symbols(IndexAlias::Nifty50);
    assert_eq!(syms, vec!["ABC", "XYZ"]);
    let entry = registry.get_entry(IndexAlias::Nifty50).unwrap();
    assert_eq!(entry.as_of, "2099-01-01");

    // Other index (not in temp_dir) still loads bundled seed
    let bank_syms = registry.get_symbols(IndexAlias::NiftyBank);
    assert!(bank_syms.len() > 5);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn refresh_failure_leaves_cache_intact_and_reports_http_status() {
    let temp_dir = tempfile_dir("refresh_fail");
    let original = IndexCacheFile {
        alias: "NIFTY_50".to_string(),
        as_of: "2020-01-01".to_string(),
        source_url: None,
        symbols: vec!["ORIGINAL1".to_string(), "ORIGINAL2".to_string()],
    };
    write_cache_atomic(&temp_dir, IndexAlias::Nifty50, &original).unwrap();

    // Spawn mock HTTP server returning 500 Internal Server Error
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let response = "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 12\r\nConnection: close\r\n\r\nServer Error";
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    let mock_base = format!("http://127.0.0.1:{port}/");
    let registry = IndexRegistry::load_with_cache_dir(Some(temp_dir.clone()));
    let client = reqwest::blocking::Client::builder().build().unwrap();

    let res = refresh_index(
        IndexAlias::Nifty50,
        &temp_dir,
        Some(&registry),
        Some(&mock_base),
        Some(&client),
    );

    assert!(res.is_err());
    match res.unwrap_err() {
        RefreshError::HttpStatus { status, .. } => assert_eq!(status, 500),
        other => panic!("expected HttpStatus 500, got {other:?}"),
    }

    server_handle.join().unwrap();

    // Verify cache file on disk is untouched
    let on_disk: IndexCacheFile = serde_json::from_str(
        &std::fs::read_to_string(temp_dir.join("nifty_50.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(on_disk.symbols, vec!["ORIGINAL1", "ORIGINAL2"]);
    assert_eq!(on_disk.as_of, "2020-01-01");

    // Verify no temporary files left behind
    let remaining_files: Vec<_> = std::fs::read_dir(&temp_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert_eq!(remaining_files.len(), 1);
    assert_eq!(
        remaining_files[0].file_name().to_string_lossy(),
        "nifty_50.json"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn refresh_success_updates_cache_and_in_memory_registry() {
    let temp_dir = tempfile_dir("refresh_success");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let csv = "Company Name,Industry,Symbol,Series,ISIN Code\r\nReliance Industries Ltd.,Energy,RELIANCE,EQ,INE002A01018\r\nTata Consultancy Services Ltd.,IT,TCS,EQ,INE467B01029\r\n";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/csv\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                csv.len(),
                csv
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    let mock_base = format!("http://127.0.0.1:{port}/");
    let registry = IndexRegistry::load_with_cache_dir(Some(temp_dir.clone()));
    let client = reqwest::blocking::Client::builder().build().unwrap();

    let res = refresh_index(
        IndexAlias::Nifty50,
        &temp_dir,
        Some(&registry),
        Some(&mock_base),
        Some(&client),
    )
    .expect("refresh should succeed");

    assert_eq!(res.symbols, vec!["RELIANCE", "TCS"]);
    assert_eq!(registry.get_symbols(IndexAlias::Nifty50), vec!["RELIANCE", "TCS"]);

    server_handle.join().unwrap();

    let on_disk: IndexCacheFile = serde_json::from_str(
        &std::fs::read_to_string(temp_dir.join("nifty_50.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(on_disk.symbols, vec!["RELIANCE", "TCS"]);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn resolve_trade_in_index_and_symbols_success() {
    let registry = IndexRegistry::load_with_cache_dir(None);
    let resolver = TradeInResolver::new(&registry);

    let scrips = vec![
        dhan_api::Scrip {
            security_id: "1333".into(),
            symbol: "HDFCBANK".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: true,
        },
        dhan_api::Scrip {
            security_id: "4963".into(),
            symbol: "ICICIBANK".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: true,
        },
        dhan_api::Scrip {
            security_id: "2885".into(),
            symbol: "RELIANCE".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: true,
        },
    ];

    // Symbols test
    let trade_in_syms = algomln::ast::TradeIn::Symbols(vec!["RELIANCE".into(), "HDFCBANK".into()]);
    let (resolved, report) = resolver.resolve_scrips(&trade_in_syms, &scrips, false).unwrap();
    assert_eq!(resolved.len(), 2);
    // Must be sorted by symbol
    assert_eq!(resolved[0].symbol, "HDFCBANK");
    assert_eq!(resolved[1].symbol, "RELIANCE");
    assert_eq!(report.total_resolved, 2);
    assert!(report.indices.is_empty());

    // Index test with allow_unresolved = true (since scrips only has 2 bank stocks)
    let trade_in_idx = algomln::ast::TradeIn::Index("NIFTY_BANK".into());
    let (resolved_idx, report_idx) = resolver.resolve_scrips(&trade_in_idx, &scrips, true).unwrap();
    assert_eq!(resolved_idx.len(), 2);
    assert_eq!(resolved_idx[0].symbol, "HDFCBANK");
    assert_eq!(resolved_idx[1].symbol, "ICICIBANK");
    assert_eq!(report_idx.indices.len(), 1);
    assert_eq!(report_idx.indices[0].alias, "NIFTY_BANK");
    assert_eq!(report_idx.indices[0].resolved_count, 2);
    assert!(!report_idx.indices[0].unresolved_symbols.is_empty());
}

#[test]
fn bare_symbol_matching_index_fails_with_hint() {
    let registry = IndexRegistry::load_with_cache_dir(None);
    let resolver = TradeInResolver::new(&registry);
    let scrips: Vec<dhan_api::Scrip> = vec![];

    let trade_in = algomln::ast::TradeIn::Symbols(vec!["NIFTY50".into()]);
    let err = resolver.resolve_scrips(&trade_in, &scrips, false).unwrap_err();

    match err {
        ResolveError::UnknownSymbol { code, name, hint } => {
            assert_eq!(code, ERR_UNKNOWN_SYMBOL);
            assert_eq!(name, "NIFTY50");
            assert!(
                hint.contains("did you mean 'INDEX NIFTY_50'?"),
                "hint was: {hint}"
            );
        }
        other => panic!("expected UnknownSymbol error with hint, got {other:?}"),
    }

    // Completely unknown symbol gets no hint
    let trade_in_unknown = algomln::ast::TradeIn::Symbols(vec!["NONEXISTENT123".into()]);
    let err2 = resolver.resolve_scrips(&trade_in_unknown, &scrips, false).unwrap_err();
    match err2 {
        ResolveError::UnknownSymbol { code, name, hint } => {
            assert_eq!(code, ERR_UNKNOWN_SYMBOL);
            assert_eq!(name, "NONEXISTENT123");
            assert!(hint.is_empty(), "expected empty hint, got: {hint}");
        }
        other => panic!("expected UnknownSymbol error without hint, got {other:?}"),
    }
}

#[test]
fn unresolved_constituent_fails_by_default_and_lists_symbols() {
    let registry = IndexRegistry::load_with_cache_dir(None);
    let resolver = TradeInResolver::new(&registry);

    // Only one symbol present for NIFTY 50
    let scrips = vec![dhan_api::Scrip {
        security_id: "2885".into(),
        symbol: "RELIANCE".into(),
        lot_size: 1,
        tick_size_paise: 5,
        instrument: "EQUITY".into(),
        is_fno: true,
    }];

    let trade_in = algomln::ast::TradeIn::Index("NIFTY_50".into());
    let err = resolver.resolve_scrips(&trade_in, &scrips, false).unwrap_err();

    match err {
        ResolveError::UnresolvedConstituents {
            code,
            index,
            count,
            symbols,
        } => {
            assert_eq!(code, ERR_UNRESOLVED_CONSTITUENT);
            assert_eq!(index, "NIFTY_50");
            assert_eq!(count, symbols.len());
            assert!(count >= 40);
            assert!(symbols.contains(&"TCS".to_string()));
        }
        other => panic!("expected UnresolvedConstituents error, got {other:?}"),
    }
}

#[test]
fn fingerprint_changes_after_refresh_and_as_of_update() {
    let temp_dir = tempfile_dir("fp_test");
    let db = temp_dir.join("catalog.sqlite");
    let mut cat = Catalog::open(&db).unwrap();

    let scrips = vec![
        dhan_api::Scrip {
            security_id: "1".into(),
            symbol: "SYM1".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: true,
        },
        dhan_api::Scrip {
            security_id: "2".into(),
            symbol: "SYM2".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: false,
        },
    ];
    cat.refresh_instruments(&scrips, 0).unwrap();

    let store = Store::new(&temp_dir);
    let bar1 = Bar { ts: 1000, open: 10.0, high: 11.0, low: 9.0, close: 10.5, volume: 100.0 };
    let bar2 = Bar { ts: 1000, open: 20.0, high: 21.0, low: 19.0, close: 20.5, volume: 200.0 };
    let parts1 = store.write_partitions("1", Tf::M1, &[bar1]).unwrap();
    let parts2 = store.write_partitions("2", Tf::M1, &[bar2]).unwrap();
    let sid1 = cat.series_for_import("1", Tf::M1).unwrap();
    let sid2 = cat.series_for_import("2", Tf::M1).unwrap();
    for p in &parts1 { cat.record_partition(sid1, &store, p).unwrap(); }
    for p in &parts2 { cat.record_partition(sid2, &store, p).unwrap(); }

    let cat_src = CatalogSource::new(store, &db);

    let symbols = vec!["SYM1".to_string(), "SYM2".to_string()];
    let tf = algomln::sim::Timeframe::M1;
    let range = algomln::sim::DateRange { from: None, to: None };

    let ds_fp = cat_src.fingerprint(&symbols, tf, range);

    // Initial resolution report with date 2026-01-01
    let rep1 = ResolutionReport {
        indices: vec![IndexResolutionReport {
            alias: "NIFTY_BANK".into(),
            display_name: "NIFTY BANK".into(),
            as_of: "2026-01-01".into(),
            constituent_count: 2,
            resolved_count: 2,
            unresolved_symbols: vec![],
        }],
        total_resolved: 2,
        allow_unresolved: false,
    };
    let run_fp1 = format_run_fingerprint(&ds_fp, &rep1);

    // After refresh: as_of date updates to 2026-01-02
    let rep2 = ResolutionReport {
        indices: vec![IndexResolutionReport {
            alias: "NIFTY_BANK".into(),
            display_name: "NIFTY BANK".into(),
            as_of: "2026-01-02".into(),
            constituent_count: 2,
            resolved_count: 2,
            unresolved_symbols: vec![],
        }],
        total_resolved: 2,
        allow_unresolved: false,
    };
    let run_fp2 = format_run_fingerprint(&ds_fp, &rep2);

    // Refresh MUST change the run fingerprint
    assert_ne!(run_fp1, run_fp2);
    assert!(run_fp1.contains("2026-01-01"));
    assert!(run_fp2.contains("2026-01-02"));

    // If constituents change, dataset hash itself changes
    let alt_symbols = vec!["SYM1".to_string()];
    let ds_fp_alt = cat_src.fingerprint(&alt_symbols, tf, range);
    assert_ne!(ds_fp.0, ds_fp_alt.0);

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn mixed_360_and_375_bar_index_streaming_through_catalog_source() {
    use algomln::sim::CandleSource;
    use chrono::{NaiveDate, TimeZone};
    use chrono_tz::Asia::Kolkata;

    let root = tempfile_dir("mixed_sessions");
    let db = root.join("catalog.sqlite");
    let mut cat = Catalog::open(&db).unwrap();

    // 1: F&O eligible (360 bars: 09:15 to 15:14)
    // 2: Non-F&O (375 bars: 09:15 to 15:29)
    let scrips = vec![
        dhan_api::Scrip {
            security_id: "1001".into(),
            symbol: "FNO_STOCK".into(),
            lot_size: 25,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: true,
        },
        dhan_api::Scrip {
            security_id: "1002".into(),
            symbol: "CASH_STOCK".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: false,
        },
    ];
    cat.refresh_instruments(&scrips, 0).unwrap();

    let store = Store::new(&root);
    let session_date = NaiveDate::from_ymd_opt(2026, 8, 3).unwrap();
    let session_start = Kolkata
        .from_local_datetime(&session_date.and_hms_opt(9, 15, 0).unwrap())
        .single()
        .unwrap()
        .timestamp();

    // Generate 360 bars for F&O stock
    let fno_bars: Vec<Bar> = (0..360)
        .map(|i| Bar {
            ts: session_start + i * 60,
            open: 100.0,
            high: 101.0,
            low: 99.0,
            close: 100.5,
            volume: 1000.0,
        })
        .collect();

    // Generate 375 bars for Cash stock
    let cash_bars: Vec<Bar> = (0..375)
        .map(|i| Bar {
            ts: session_start + i * 60,
            open: 50.0,
            high: 51.0,
            low: 49.0,
            close: 50.5,
            volume: 500.0,
        })
        .collect();

    let parts_fno = store.write_partitions("1001", Tf::M1, &fno_bars).unwrap();
    let parts_cash = store.write_partitions("1002", Tf::M1, &cash_bars).unwrap();

    let sid_fno = cat.series_for_import("1001", Tf::M1).unwrap();
    let sid_cash = cat.series_for_import("1002", Tf::M1).unwrap();

    for p in &parts_fno {
        cat.record_partition(sid_fno, &store, p).unwrap();
    }
    for p in &parts_cash {
        cat.record_partition(sid_cash, &store, p).unwrap();
    }

    let fno_end = session_start + 359 * 60;
    let cash_end = session_start + 374 * 60;

    cat.record_session(sid_fno, session_date, session_start, fno_end, 360).unwrap();
    cat.record_session(sid_cash, session_date, session_start, cash_end, 375).unwrap();

    cat.record_coverage("1001", Tf::M1, session_start, fno_end, "data").unwrap();
    cat.record_coverage("1002", Tf::M1, session_start, cash_end, "data").unwrap();
    drop(cat);

    let source = CatalogSource::new(Store::new(&root), &db);
    let symbols = vec!["CASH_STOCK".to_string(), "FNO_STOCK".to_string()];
    let range = algomln::sim::DateRange {
        from: Some(session_start),
        to: Some(cash_end),
    };

    let stream = source
        .open(&symbols, algomln::sim::Timeframe::M1, range)
        .expect("open stream must succeed");

    let items: Vec<(String, algomln::indicators::Candle)> =
        stream.map(|r| r.expect("candle item")).collect();

    // Total candles must be 360 (FNO) + 375 (CASH) = 735
    assert_eq!(items.len(), 360 + 375);

    // Monotonic timestamp check
    for w in items.windows(2) {
        assert!(
            w[0].1.timestamp <= w[1].1.timestamp,
            "candles must be non-decreasing in timestamp"
        );
        if w[0].1.timestamp == w[1].1.timestamp {
            // CASH_STOCK comes before FNO_STOCK due to alphabetical ordering
            assert_eq!(w[0].0, "CASH_STOCK");
            assert_eq!(w[1].0, "FNO_STOCK");
        }
    }

    // Verify after timestamp > fno_end, only CASH_STOCK is received
    let late_items: Vec<_> = items
        .iter()
        .filter(|(_, c)| c.timestamp > fno_end)
        .collect();
    assert_eq!(late_items.len(), 15);
    for (sym, _) in late_items {
        assert_eq!(sym, "CASH_STOCK");
    }

    let _ = std::fs::remove_dir_all(root);
}

fn tempfile_dir(prefix: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("algomln_test_{}_{}", prefix, std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}
