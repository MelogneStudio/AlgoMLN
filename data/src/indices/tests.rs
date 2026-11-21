use super::*;
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

fn tempfile_dir(prefix: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!("algomln_test_{}_{}", prefix, std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    dir
}
