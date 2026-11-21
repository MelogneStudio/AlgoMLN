use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;

use algomln_data::{
    write_cache_atomic, Bar, Catalog, IndexAlias, IndexCacheFile, IndexRegistry,
    Store, Tf,
};

#[test]
fn test_index_list_and_show() {
    let temp_dir = tempfile_dir("cli_list_show");
    let cache_dir = temp_dir.join("indices");

    let reg = IndexRegistry::load_with_cache_dir(Some(cache_dir.clone()));
    let meta = reg.list_metadata();
    assert_eq!(meta.len(), 22);

    // Verify nifty_bank entry
    let bank = reg.get_entry(IndexAlias::NiftyBank).unwrap();
    assert_eq!(bank.dsl_keyword, "NIFTY_BANK");
    assert!(!bank.symbols.is_empty());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_index_refresh_all_with_mock_server() {
    let temp_dir = tempfile_dir("cli_refresh");
    let cache_dir = temp_dir.join("indices");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();

    let server_handle = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let csv = "Company Name,Industry,Symbol,Series,ISIN Code\r\nBank1,Financial,HDFCBANK,EQ,INE040A01034\r\nBank2,Financial,ICICIBANK,EQ,INE090A01021\r\n";
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
    let reg = IndexRegistry::load_with_cache_dir(Some(cache_dir.clone()));

    let refreshed = algomln_data::refresh_index(
        IndexAlias::NiftyBank,
        &cache_dir,
        Some(&reg),
        Some(&mock_base),
        None,
    )
    .unwrap();

    assert_eq!(refreshed.symbols, vec!["HDFCBANK", "ICICIBANK"]);

    // Old cache intact on failure test
    let fail_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let fail_port = fail_listener.local_addr().unwrap().port();
    let fail_server = thread::spawn(move || {
        if let Ok((mut stream, _)) = fail_listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\nConnection: close\r\n\r\nNot Found";
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.flush();
        }
    });

    let fail_base = format!("http://127.0.0.1:{fail_port}/");
    let fail_res = algomln_data::refresh_index(
        IndexAlias::NiftyBank,
        &cache_dir,
        Some(&reg),
        Some(&fail_base),
        None,
    );
    assert!(fail_res.is_err());

    // Nifty bank cache on disk still has previous successful symbols
    let on_disk: IndexCacheFile = serde_json::from_str(
        &std::fs::read_to_string(cache_dir.join("nifty_bank.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(on_disk.symbols, vec!["HDFCBANK", "ICICIBANK"]);

    server_handle.join().unwrap();
    fail_server.join().unwrap();
    let _ = std::fs::remove_dir_all(temp_dir);
}

#[test]
fn test_trade_in_index_reproducibility_and_bit_identical_fingerprints() {
    let temp_dir = tempfile_dir("reproducibility");
    let db = temp_dir.join("catalog.sqlite");
    let mut cat = Catalog::open(&db).unwrap();

    let scrips = vec![
        dhan_api::Scrip {
            security_id: "100".into(),
            symbol: "HDFCBANK".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: true,
        },
        dhan_api::Scrip {
            security_id: "200".into(),
            symbol: "ICICIBANK".into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: true,
        },
    ];
    cat.refresh_instruments(&scrips, 0).unwrap();

    let store = Store::new(&temp_dir);
    let bar1 = Bar { ts: 1000, open: 1500.0, high: 1510.0, low: 1495.0, close: 1505.0, volume: 5000.0 };
    let bar2 = Bar { ts: 1000, open: 1000.0, high: 1005.0, low: 995.0, close: 1002.0, volume: 3000.0 };
    let p1 = store.write_partitions("100", Tf::M1, &[bar1]).unwrap();
    let p2 = store.write_partitions("200", Tf::M1, &[bar2]).unwrap();
    let s1 = cat.series_for_import("100", Tf::M1).unwrap();
    let s2 = cat.series_for_import("200", Tf::M1).unwrap();
    for p in &p1 { cat.record_partition(s1, &store, p).unwrap(); }
    for p in &p2 { cat.record_partition(s2, &store, p).unwrap(); }

    let cache_dir = temp_dir.join("indices");
    let custom_cache = IndexCacheFile {
        alias: "NIFTY_BANK".into(),
        as_of: "2026-11-01".into(),
        source_url: None,
        // Overlap / duplicate symbols test in index cache
        symbols: vec!["ICICIBANK".into(), "HDFCBANK".into(), "HDFCBANK".into()],
    };
    write_cache_atomic(&cache_dir, IndexAlias::NiftyBank, &custom_cache).unwrap();

    let reg = IndexRegistry::load_with_cache_dir(Some(cache_dir.clone()));
    let resolver = algomln_data::TradeInResolver::new(&reg);

    let trade_in = algomln::ast::TradeIn::Index("NIFTY_BANK".into());
    let (resolved1, rep1) = resolver.resolve_catalog(&trade_in, &cat, false).unwrap();
    let (resolved2, rep2) = resolver.resolve_catalog(&trade_in, &cat, false).unwrap();

    // Deduplication across repeating symbols verified
    assert_eq!(resolved1.len(), 2);
    assert_eq!(resolved1[0].symbol, "HDFCBANK");
    assert_eq!(resolved1[1].symbol, "ICICIBANK");
    assert_eq!(resolved1, resolved2);

    let cat_src = algomln_data::CatalogSource::new(Store::new(&temp_dir), &db);
    use algomln::sim::CandleSource;
    let syms: Vec<String> = resolved1.iter().map(|s| s.symbol.clone()).collect();
    let range = algomln::sim::DateRange { from: None, to: None };

    let ds_fp1 = cat_src.fingerprint(&syms, algomln::sim::Timeframe::M1, range);
    let ds_fp2 = cat_src.fingerprint(&syms, algomln::sim::Timeframe::M1, range);
    assert_eq!(ds_fp1, ds_fp2);

    let run_fp1 = algomln_data::format_run_fingerprint(&ds_fp1, &rep1);
    let run_fp2 = algomln_data::format_run_fingerprint(&ds_fp2, &rep2);
    // Two runs with same inputs and same index cache must have bit-identical fingerprints
    assert_eq!(run_fp1, run_fp2);

    // Refreshing cache with new date changes fingerprint
    let refreshed_cache = IndexCacheFile {
        alias: "NIFTY_BANK".into(),
        as_of: "2026-11-21".into(),
        source_url: None,
        symbols: vec!["HDFCBANK".into(), "ICICIBANK".into()],
    };
    write_cache_atomic(&cache_dir, IndexAlias::NiftyBank, &refreshed_cache).unwrap();
    let reg_new = IndexRegistry::load_with_cache_dir(Some(cache_dir.clone()));
    let resolver_new = algomln_data::TradeInResolver::new(&reg_new);
    let (_, rep_new) = resolver_new.resolve_catalog(&trade_in, &cat, false).unwrap();
    let run_fp_refreshed = algomln_data::format_run_fingerprint(&ds_fp1, &rep_new);

    assert_ne!(run_fp1, run_fp_refreshed);
    assert!(run_fp_refreshed.contains("2026-11-21"));

    let _ = std::fs::remove_dir_all(temp_dir);
}

fn tempfile_dir(prefix: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("algomln_cli_test_{}_{}", prefix, std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}
