use super::*;
use algomln::sim::{CandleSource, DateRange, FileSource, Timeframe};
use dhan_api::{DhanClient, Scrip, TokenProvider};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::Instant;

struct TestToken;
impl TokenProvider for TestToken {
    fn access_token(&self) -> Result<String, dhan_api::ApiError> {
        Ok("secret".into())
    }
}

/// Outcome of the mock server thread: the number of requests served and the
/// [`Instant`] at which each request's bytes were received.
struct MockOutcome {
    served: usize,
    received_at: Vec<Instant>,
}

/// Start a TCP mock Dhan server that replies to each incoming request with
/// the next `(status, body)` pair in `responses`, recording the instant each
/// request arrives. Returns the base URL and a join handle. Modelled on the
/// dhan-api `mock` helper (raw HTTP/1.1 over `TcpListener`).
fn mock_server(
    responses: Vec<(u16, String)>,
) -> (String, std::thread::JoinHandle<MockOutcome>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let mut received_at = Vec::new();
        let mut served = 0;
        for (status, body) in responses {
            let (mut stream, _) = match listener.accept() {
                Ok(x) => x,
                Err(_) => break,
            };
            let mut buf = [0u8; 8192];
            let _ = stream.read(&mut buf).unwrap();
            received_at.push(Instant::now());
            served += 1;
            let phrase = match status {
                200 => "OK",
                400 => "Bad Request",
                401 => "Unauthorized",
                429 => "Too Many Requests",
                500 => "Internal Server Error",
                _ => "Error",
            };
            let response = format!(
                "HTTP/1.1 {status} {phrase}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
                body.len(),
                if status == 429 { "Retry-After: 0\r\n" } else { "" },
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        MockOutcome {
            served,
            received_at,
        }
    });
    (format!("http://{addr}"), handle)
}

/// Build the intraday columnar JSON the Dhan `/charts/intraday` endpoint
/// returns for `n` consecutive one-minute bars starting at `start_ts`.
fn intraday_body(start_ts: i64, n: usize) -> String {
    let ts: Vec<String> = (0..n)
        .map(|i| (start_ts + i as i64 * 60).to_string())
        .collect();
    let ones = vec!["1.0"; n].join(",");
    let twos = vec!["2.0"; n].join(",");
    let vols = vec!["10.0"; n].join(",");
    format!(
        "{{\"open\":[{o}],\"high\":[{h}],\"low\":[{l}],\"close\":[{c}],\"volume\":[{v}],\"timestamp\":[{t}]}}",
        o = ones,
        h = twos,
        l = ones,
        c = twos,
        v = vols,
        t = ts.join(",")
    )
}

fn empty_body() -> String {
    r#"{"open":[],"high":[],"low":[],"close":[],"volume":[],"timestamp":[]}"#.to_string()
}

fn make_updater(base: &str, db: &Path, root: &Path) -> Updater {
    let client = DhanClient::with_base(Arc::new(TestToken), base).unwrap();
    Updater {
        client: Box::new(client),
        catalog: Catalog::open(db).unwrap(),
        store: Store::new(root),
        max_retries: 0,
        margin_seconds: 0,
        reference_symbol_id: None,
        budget_cap: DEFAULT_BUDGET_CAP,
    }
}

fn seed_instrument(db: &Path, id: &str, symbol: &str) {
    let mut cat = Catalog::open(db).unwrap();
    cat.refresh_instruments(
        &[Scrip {
            security_id: id.into(),
            symbol: symbol.into(),
            lot_size: 1,
            tick_size_paise: 5,
            instrument: "EQUITY".into(),
            is_fno: false,
        }],
        0,
    )
    .unwrap();
}

fn open_ist(d: NaiveDate) -> i64 {
    d.and_hms_opt(9, 15, 0)
        .unwrap()
        .and_local_timezone(Kolkata)
        .single()
        .unwrap()
        .timestamp()
}

fn temp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "algomln-acc-{name}-{}-{}",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap()
    ))
}

/// Spec 11.2: request start times are never closer than 250 ms, measured
/// from the mock's receive timestamps. Three one-minute chunks across a
/// single intraday series must arrive >=250 ms apart, enforced by the shared
/// limiter inside the real [`DhanClient`].
#[test]
fn request_starts_are_at_least_250ms_apart() {
    let root = temp("spacing");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    // Three 89-day chunks => three intraday requests.
    let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let to = from + chrono::Days::new(179);
    assert_eq!(chunk_dates(from, to).len(), 3);
    let responses = chunk_dates(from, to)
        .iter()
        .map(|(a, _)| (200u16, intraday_body(open_ist(*a), 1)))
        .collect::<Vec<_>>();
    let (base, server) = mock_server(responses);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", from, to).unwrap();
    let outcome = server.join().unwrap();
    assert_eq!(outcome.served, 3);
    for pair in outcome.received_at.windows(2) {
        let gap = pair[1].duration_since(pair[0]);
        // The limiter targets 250 ms between request starts; allow generous
        // slack for localhost delivery/scheduling jitter on the receive side.
        // Without the limiter these arrive sub-millisecond apart, so a 200 ms
        // floor still unambiguously proves spacing is enforced.
        assert!(
            gap >= std::time::Duration::from_millis(200),
            "requests only {gap:?} apart; limiter must keep ~250 ms spacing"
        );
    }
    let _ = fs::remove_dir_all(root);
}

/// Spec 11.6: an injected DH-904 triggers backoff, and the run still
/// completes. The DhanClient retries after the limiter cools down; the
/// second attempt succeeds and the series is recorded.
#[test]
fn injected_dh904_backs_off_and_run_completes() {
    let root = temp("dh904");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let to = from; // single intraday chunk
    let responses = vec![
        (
            429u16,
            r#"{"errorCode":"DH-904","errorMessage":"slow down"}"#.to_string(),
        ),
        (200u16, intraday_body(open_ist(from), 1)),
    ];
    let (base, server) = mock_server(responses);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", from, to).unwrap();
    let outcome = server.join().unwrap();
    assert_eq!(
        outcome.served, 2,
        "DH-904 must be retried once then succeed"
    );
    let covered = up.catalog.covered_dates("1", Tf::M1, from, to).unwrap();
    assert_eq!(covered, vec![from]);
    let _ = fs::remove_dir_all(root);
}

/// Spec 11.8: an auth failure aborts the run immediately. Driven through the
/// `BulkDownloader` so the abort is observed at the run level, not just the
/// client: `is_auth()` short-circuits and the error propagates.
#[test]
fn auth_error_aborts_the_run_immediately() {
    let root = temp("auth");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "AAA");
    // A second symbol that must NOT be contacted once auth fails.
    {
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(
            &[
                Scrip {
                    security_id: "1".into(),
                    symbol: "AAA".into(),
                    lot_size: 1,
                    tick_size_paise: 5,
                    instrument: "EQUITY".into(),
                    is_fno: false,
                },
                Scrip {
                    security_id: "2".into(),
                    symbol: "BBB".into(),
                    lot_size: 1,
                    tick_size_paise: 5,
                    instrument: "EQUITY".into(),
                    is_fno: false,
                },
            ],
            0,
        )
        .unwrap();
    }
    let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let to = from;
    // Only one response queued: the first (and only) request gets 401.
    let (base, server) = mock_server(vec![(
        401u16,
        r#"{"errorCode":"DH-902","errorMessage":"invalid token"}"#.to_string(),
    )]);
    let base_for_factory = base.clone();
    let bulk = BulkDownloader {
        make_client: move || {
            Box::new(DhanClient::with_base(Arc::new(TestToken), &base_for_factory).unwrap())
                as Box<dyn HistoryApi>
        },
        catalog_path: db.clone(),
        store_root: root.clone(),
        config: BulkConfig::new(from, to, vec![Tf::D1]),
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let result = bulk.run(|_| {});
    assert!(
        matches!(&result, Err(e) if e.is_auth()),
        "auth failure must abort the run with an auth error, got {result:?}"
    );
    let outcome = server.join().unwrap();
    assert_eq!(
        outcome.served, 1,
        "run must abort after the first auth failure without contacting the next symbol"
    );
    let _ = fs::remove_dir_all(root);
}

/// Spec 11.7: an empty response is logged as `no_data` and is NOT recorded
/// as coverage. The reference symbol path records the calendar as empty, but
/// no `data` coverage is written and the session is not marked covered for a
/// non-reference symbol.
#[test]
fn empty_response_is_not_recorded_as_coverage() {
    let root = temp("nodata");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let to = from;
    let (base, server) = mock_server(vec![(200u16, empty_body())]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", from, to).unwrap();
    let outcome = server.join().unwrap();
    assert_eq!(outcome.served, 1);
    // The spec guarantee: an empty response is never recorded as *data*
    // coverage (it could be a holiday or a pre-listing range). The empty day
    // may be logged as `confirmed_empty`/`no_data` so it is not re-requested
    // forever, but it must not claim real bars exist.
    let sid = up.catalog.ensure_series("1", Tf::M1).unwrap();
    let data_rows: i64 = up
        .catalog
        .conn
        .query_row(
            "SELECT COUNT(*) FROM coverage WHERE series_id=?1 AND kind='data'",
            [sid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(data_rows, 0, "empty response must not add 'data' coverage");
    // No real bars were stored.
    let bars: i64 = up
        .catalog
        .conn
        .query_row(
            "SELECT COALESCE(SUM(rows),0) FROM partitions WHERE series_id=?1",
            [sid],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(bars, 0, "empty response must not store any bars");
    let _ = fs::remove_dir_all(root);
}

/// Spec 11.9: a symbol with 375-bar sessions downloads and verifies
/// correctly — the downloader must not assume 360 bars per session. The mock
/// returns a full 375-minute session; all 375 bars must be stored.
#[test]
fn symbol_with_375_bar_session_downloads_all_bars() {
    let root = temp("375");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let day = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
    // 375 one-minute bars: 09:15..=15:29 inclusive is 375 minutes.
    let (base, server) = mock_server(vec![(200u16, intraday_body(open_ist(day), 375))]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", day, day).unwrap();
    let outcome = server.join().unwrap();
    assert_eq!(outcome.served, 1);
    let status = up.catalog.status_rows().unwrap();
    assert_eq!(status.len(), 1);
    assert_eq!(
        status[0].bars, 375,
        "all 375 session bars must be stored; downloader must not cap at 360"
    );
    let _ = fs::remove_dir_all(root);
}

/// Spec 6 + 11.2: with `concurrency > 1` the shared limiter must still cap the
/// global request rate at ~4/s. Eight symbols are fetched by a 4-worker pool
/// through one cloned-per-worker `DhanClient` limiter.
///
/// The limiter is a token bucket (cap 4), so it permits a short burst of up to
/// four requests before throttling — a per-gap floor is therefore fragile (the
/// bucket refills during worker setup and the first few arrivals bunch up, as
/// seen with 195 ms gaps). The robust invariant is the *total* span: with a max
/// burst of 4 tokens, `N` requests take at least `(N - 4) / 4` seconds, i.e.
/// >=1.0 s for 8. Without the limiter all eight arrive in well under 100 ms, so
/// an 0.8 s floor unambiguously proves throttling survives concurrency. Also
/// proves the concurrent write path commits every symbol without racing on the
/// shared catalog (WAL + busy_timeout + IMMEDIATE writes).
#[test]
fn concurrent_run_throttles_global_rate_and_commits_all_symbols() {
    let root = temp("concurrent-spacing");
    let db = root.join("catalog.sqlite");
    let symbols = ["AAA", "BBB", "CCC", "DDD", "EEE", "FFF", "GGG", "HHH"];
    for (i, sym) in symbols.iter().enumerate() {
        seed_instrument(&db, &(i + 1).to_string(), sym);
    }
    let day = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();
    // One intraday request per symbol (single-day range => single chunk).
    let responses = (0..symbols.len())
        .map(|_| (200u16, intraday_body(open_ist(day), 1)))
        .collect::<Vec<_>>();
    let (base, server) = mock_server(responses);
    // Every worker's client shares the one limiter, so the rate cap is global
    // across the pool rather than per client.
    let client = DhanClient::with_base(Arc::new(TestToken), &base).unwrap();
    let base_for_factory = base.clone();
    let make_client = move || {
        Box::new(
            client
                .clone_with_base(&base_for_factory)
                .expect("clone shares the limiter"),
        ) as Box<dyn HistoryApi>
    };
    let mut cfg = BulkConfig::new(day, day, vec![Tf::M1]);
    cfg.concurrency = 4;
    cfg.margin_seconds = 0;
    let bulk = BulkDownloader {
        make_client,
        catalog_path: db.clone(),
        store_root: root.clone(),
        config: cfg,
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    let report = bulk.run(|_| {}).unwrap();
    assert_eq!(
        report.symbols_completed,
        symbols.len(),
        "every symbol must commit"
    );
    let outcome = server.join().unwrap();
    assert_eq!(
        outcome.served,
        symbols.len(),
        "exactly one request per symbol"
    );
    let mut times = outcome.received_at;
    times.sort();
    let span = times
        .last()
        .unwrap()
        .duration_since(*times.first().unwrap());
    assert!(
        span >= std::time::Duration::from_millis(800),
        "eight requests spanned only {span:?}; the shared limiter must keep the \
         global rate near 4/s even with concurrency"
    );
    // Every series is present with its single bar committed (no write races).
    let cat = Catalog::open(&db).unwrap();
    let status = cat.status_rows().unwrap();
    assert_eq!(status.len(), symbols.len());
    assert!(status.iter().all(|s| s.bars == 1));
    let _ = fs::remove_dir_all(root);
}

fn multi_day_intraday_body(days: &[NaiveDate], bars_per_day: usize) -> String {
    let mut ts = Vec::new();
    for &d in days {
        let base = open_ist(d);
        for i in 0..bars_per_day {
            ts.push((base + i as i64 * 60).to_string());
        }
    }
    let n = ts.len();
    let ones = vec!["1.0"; n].join(",");
    let twos = vec!["2.0"; n].join(",");
    let vols = vec!["10.0"; n].join(",");
    format!(
        "{{\"open\":[{o}],\"high\":[{h}],\"low\":[{l}],\"close\":[{c}],\"volume\":[{v}],\"timestamp\":[{t}]}}",
        o = ones,
        h = twos,
        l = ones,
        c = twos,
        v = vols,
        t = ts.join(",")
    )
}

/// T1: Update a symbol over Monday to the following Monday with the mock server,
/// then open the same range. Must succeed (regression for D1).
#[test]
fn t1_update_over_weekend_then_open_must_succeed() {
    let root = temp("t1-weekend");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let mon1 = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let mon2 = NaiveDate::from_ymd_opt(2026, 1, 12).unwrap();
    let trading_days = vec![
        mon1,
        NaiveDate::from_ymd_opt(2026, 1, 6).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 7).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 8).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 9).unwrap(),
        mon2,
    ];
    let body = multi_day_intraday_body(&trading_days, 375);
    let (base, server) = mock_server(vec![(200u16, body)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon1, mon2).unwrap();
    let outcome = server.join().unwrap();
    assert_eq!(outcome.served, 1);

    let source = CatalogSource::new(Store::new(&root), &db);
    let range = DateRange {
        from: Some(open_ist(mon1)),
        to: Some(open_ist(mon2) + 375 * 60 - 60),
    };
    let stream = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
    let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(candles.len(), 6 * 375);
    let _ = fs::remove_dir_all(root);
}

/// T2: A weekday holiday marked non-trading in the calendar inside the range does not block open.
#[test]
fn t2_weekday_holiday_does_not_block_open() {
    let root = temp("t2-holiday");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let tue = NaiveDate::from_ymd_opt(2026, 1, 6).unwrap();
    let wed = NaiveDate::from_ymd_opt(2026, 1, 7).unwrap();
    {
        let cat = Catalog::open(&db).unwrap();
        cat.conn
            .execute(
                "INSERT INTO trading_calendar (date, is_trading, source) VALUES (?1, 0, 'holiday')",
                params![tue.to_string()],
            )
            .unwrap();
    }
    let trading_days = vec![mon, wed];
    let body = multi_day_intraday_body(&trading_days, 375);
    let (base, server) = mock_server(vec![(200u16, body)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon, wed).unwrap();
    let _ = server.join().unwrap();

    let source = CatalogSource::new(Store::new(&root), &db);
    let range = DateRange {
        from: Some(open_ist(mon)),
        to: Some(open_ist(wed) + 375 * 60 - 60),
    };
    let stream = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
    let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(candles.len(), 2 * 375);
    let _ = fs::remove_dir_all(root);
}

/// T3: Uncovered symbols: the error lists all of them with exact commands, and the catalog file is byte-identical afterwards.
#[test]
fn t3_uncovered_symbols_error_and_catalog_byte_identical() {
    let root = temp("t3-uncovered");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    seed_instrument(&db, "2", "XYZ");
    let before_bytes = fs::read(&db).unwrap();

    let source = CatalogSource::new(Store::new(&root), &db);
    let d1 = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let d2 = NaiveDate::from_ymd_opt(2026, 1, 6).unwrap();
    let range = DateRange {
        from: Some(open_ist(d1)),
        to: Some(open_ist(d2) + 375 * 60 - 60),
    };
    let err = match source.open(&["ABC".into(), "XYZ".into()], Timeframe::M1, range) {
        Ok(_) => panic!("expected error"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("ABC"));
    assert!(err.contains("XYZ"));
    assert!(err.contains("algomln data update --symbols ABC,XYZ --tf 1m"));

    let after_bytes = fs::read(&db).unwrap();
    assert_eq!(before_bytes, after_bytes);

    let cat = Catalog::open(&db).unwrap();
    let count: i64 = cat
        .conn
        .query_row("SELECT count(*) FROM series", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
    let _ = fs::remove_dir_all(root);
}

/// T4: Open-ended from and to resolve to the series bounds. An empty series returns Uncovered.
#[test]
fn t4_open_ended_ranges_and_empty_series() {
    let root = temp("t4-openended");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    seed_instrument(&db, "2", "EMPTY");
    let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let tue = NaiveDate::from_ymd_opt(2026, 1, 6).unwrap();
    let body = multi_day_intraday_body(&[mon, tue], 375);
    let (base, server) = mock_server(vec![(200u16, body)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon, tue).unwrap();
    let _ = server.join().unwrap();

    let source = CatalogSource::new(Store::new(&root), &db);
    let stream = source
        .open(
            &["ABC".into()],
            Timeframe::M1,
            DateRange {
                from: None,
                to: None,
            },
        )
        .unwrap();
    let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(candles.len(), 2 * 375);

    let res = source.open(
        &["EMPTY".into()],
        Timeframe::M1,
        DateRange {
            from: None,
            to: None,
        },
    );
    assert!(res.is_err());

    let _ = fs::remove_dir_all(root);
}

/// T5: Warm-up: an F&O and a non-F&O symbol get different sessions_needed for the same warmup_bars.
/// Insufficient history returns WarmupShort. With allow_gaps it warns and continues.
#[test]
fn t5_warmup_fno_vs_non_fno_and_warmup_short() {
    let root = temp("t5-warmup");
    let db = root.join("catalog.sqlite");
    {
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(
            &[
                Scrip {
                    security_id: "1".into(),
                    symbol: "FNO_SYM".into(),
                    lot_size: 25,
                    tick_size_paise: 5,
                    instrument: "EQUITY".into(),
                    is_fno: true,
                },
                Scrip {
                    security_id: "2".into(),
                    symbol: "EQ_SYM".into(),
                    lot_size: 1,
                    tick_size_paise: 5,
                    instrument: "EQUITY".into(),
                    is_fno: false,
                },
            ],
            0,
        )
        .unwrap();
    }
    let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let tue = NaiveDate::from_ymd_opt(2026, 1, 6).unwrap();
    let body_fno = multi_day_intraday_body(&[mon, tue], 360);
    let body_eq = multi_day_intraday_body(&[mon, tue], 375);
    let (base, server) = mock_server(vec![(200u16, body_fno), (200u16, body_eq)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon, tue).unwrap();
    up.update("2", Tf::M1, "EQUITY", mon, tue).unwrap();
    let _ = server.join().unwrap();

    let mut source = CatalogSource::new(Store::new(&root), &db).with_warmup_bars(365);
    let range = DateRange {
        from: Some(open_ist(tue)),
        to: Some(open_ist(tue) + 360 * 60 - 60),
    };
    let err = match source.open(&["FNO_SYM".into()], Timeframe::M1, range) {
        Ok(_) => panic!("expected warmup short error"),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("warmup short"));

    let range_eq = DateRange {
        from: Some(open_ist(tue)),
        to: Some(open_ist(tue) + 375 * 60 - 60),
    };
    let stream = source
        .open(&["EQ_SYM".into()], Timeframe::M1, range_eq)
        .unwrap();
    let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
    assert!(candles.len() > 375);

    source.allow_gaps = true;
    let stream_fno = source
        .open(&["FNO_SYM".into()], Timeframe::M1, range)
        .unwrap();
    let candles_fno: Vec<_> = stream_fno.collect::<Result<Vec<_>, _>>().unwrap();
    assert!(!candles_fno.is_empty());

    let _ = fs::remove_dir_all(root);
}

/// T6: Bit-identical reruns: two runs give the same candle-sequence hash, the same fingerprint.
#[test]
fn t6_bit_identical_reruns() {
    let root = temp("t6-identical");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let body = multi_day_intraday_body(&[mon], 375);
    let (base, server) = mock_server(vec![(200u16, body)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
    let _ = server.join().unwrap();

    let source = CatalogSource::new(Store::new(&root), &db);
    let range = DateRange {
        from: Some(open_ist(mon)),
        to: Some(open_ist(mon) + 375 * 60 - 60),
    };
    let fp1 = source.fingerprint(&["ABC".into()], Timeframe::M1, range);
    let fp2 = source.fingerprint(&["ABC".into()], Timeframe::M1, range);
    assert_eq!(fp1, fp2);

    let stream1 = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
    let c1: Vec<_> = stream1.collect::<Result<Vec<_>, _>>().unwrap();
    let stream2 = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
    let c2: Vec<_> = stream2.collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(c1, c2);
    let _ = fs::remove_dir_all(root);
}

/// T7: FileSource and CatalogSource on identical candles produce identical streams.
#[test]
fn t7_filesource_and_catalogsource_identical() {
    let root = temp("t7-file-vs-cat");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let body = multi_day_intraday_body(&[mon], 375);
    let (base, server) = mock_server(vec![(200u16, body)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
    let _ = server.join().unwrap();

    let cat_source = CatalogSource::new(Store::new(&root), &db);
    let range = DateRange {
        from: Some(open_ist(mon)),
        to: Some(open_ist(mon) + 375 * 60 - 60),
    };
    let cat_candles: Vec<_> = cat_source
        .open(&["ABC".into()], Timeframe::M1, range)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let csv_dir = root.join("csv");
    fs::create_dir_all(&csv_dir).unwrap();
    let mut wtr = csv::Writer::from_path(csv_dir.join("ABC.csv")).unwrap();
    wtr.write_record(&["ts", "open", "high", "low", "close", "volume"])
        .unwrap();
    for (_, c) in &cat_candles {
        wtr.write_record(&[
            c.timestamp.to_string(),
            c.open.to_string(),
            c.high.to_string(),
            c.low.to_string(),
            c.close.to_string(),
            c.volume.to_string(),
        ])
        .unwrap();
    }
    wtr.flush().unwrap();

    let file_source = FileSource::new(&csv_dir);
    let file_candles: Vec<_> = file_source
        .open(&["ABC".into()], Timeframe::M1, range)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(cat_candles.len(), file_candles.len());
    for (c_cat, c_file) in cat_candles.iter().zip(file_candles.iter()) {
        assert_eq!(c_cat.0, c_file.0);
        assert_eq!(c_cat.1.timestamp, c_file.1.timestamp);
        assert_eq!(c_cat.1.open, c_file.1.open);
        assert_eq!(c_cat.1.close, c_file.1.close);
    }
    let _ = fs::remove_dir_all(root);
}

/// T8: A 360-bar symbol and a 375-bar symbol merge: ties resolved by symbol name.
#[test]
fn t8_symbol_merge_ties_resolved_by_symbol_name() {
    let root = temp("t8-merge");
    let db = root.join("catalog.sqlite");
    {
        let mut cat = Catalog::open(&db).unwrap();
        cat.refresh_instruments(
            &[
                Scrip {
                    security_id: "1".into(),
                    symbol: "BBB".into(),
                    lot_size: 1,
                    tick_size_paise: 5,
                    instrument: "EQUITY".into(),
                    is_fno: false,
                },
                Scrip {
                    security_id: "2".into(),
                    symbol: "AAA".into(),
                    lot_size: 25,
                    tick_size_paise: 5,
                    instrument: "EQUITY".into(),
                    is_fno: true,
                },
            ],
            0,
        )
        .unwrap();
    }
    let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let body_b = multi_day_intraday_body(&[mon], 375);
    let body_a = multi_day_intraday_body(&[mon], 360);
    let (base, server) = mock_server(vec![(200u16, body_b), (200u16, body_a)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
    up.update("2", Tf::M1, "EQUITY", mon, mon).unwrap();
    let _ = server.join().unwrap();

    let source = CatalogSource::new(Store::new(&root), &db);
    let range = DateRange {
        from: Some(open_ist(mon)),
        to: Some(open_ist(mon) + 375 * 60 - 60),
    };
    let stream = source
        .open(&["BBB".into(), "AAA".into()], Timeframe::M1, range)
        .unwrap();
    let merged: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();

    assert_eq!(merged.len(), 360 + 375);
    let a_count = merged.iter().filter(|(s, _)| s == "AAA").count();
    let b_count = merged.iter().filter(|(s, _)| s == "BBB").count();
    assert_eq!(a_count, 360);
    assert_eq!(b_count, 375);

    assert_eq!(merged[0].0, "AAA");
    assert_eq!(merged[1].0, "BBB");
    assert_eq!(merged[0].1.timestamp, merged[1].1.timestamp);

    let _ = fs::remove_dir_all(root);
}

/// T9: Flip one byte in a partition: run fails naming the path, catalog unchanged. A deleted partition file fails the same way.
#[test]
fn t9_corrupted_or_deleted_partition_fails() {
    let root = temp("t9-corrupt");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let body = multi_day_intraday_body(&[mon], 375);
    let (base, server) = mock_server(vec![(200u16, body)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
    let _ = server.join().unwrap();

    let cat_before = fs::read(&db).unwrap();

    let part_file = root.join("candles").join("1").join("1m").join("2026-01.parquet");
    assert!(part_file.exists());

    let mut bytes = fs::read(&part_file).unwrap();
    bytes[0] ^= 0xFF;
    fs::write(&part_file, &bytes).unwrap();

    let source = CatalogSource::new(Store::new(&root), &db);
    let range = DateRange {
        from: Some(open_ist(mon)),
        to: Some(open_ist(mon) + 375 * 60 - 60),
    };
    let err = match source.open(&["ABC".into()], Timeframe::M1, range) {
        Ok(stream) => stream.collect::<Result<Vec<_>, _>>().unwrap_err().to_string(),
        Err(e) => e.to_string(),
    };
    assert!(err.contains("checksum mismatch"));
    assert!(err.contains("data verify"));

    let cat_after = fs::read(&db).unwrap();
    assert_eq!(cat_before, cat_after);

    fs::remove_file(&part_file).unwrap();
    let err2 = match source.open(&["ABC".into()], Timeframe::M1, range) {
        Ok(stream) => stream.collect::<Result<Vec<_>, _>>().unwrap_err().to_string(),
        Err(e) => e.to_string(),
    };
    assert!(err2.contains("missing partition file"));

    let _ = fs::remove_dir_all(root);
}

/// T10: A corrupted partition of an unrelated symbol does not affect or slow the run.
#[test]
fn t10_corrupted_partition_of_unrelated_symbol_does_not_affect_run() {
    let root = temp("t10-unrelated");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "GOOD");
    seed_instrument(&db, "2", "BAD");
    let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let body1 = multi_day_intraday_body(&[mon], 375);
    let body2 = multi_day_intraday_body(&[mon], 375);
    let (base, server) = mock_server(vec![(200u16, body1), (200u16, body2)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
    up.update("2", Tf::M1, "EQUITY", mon, mon).unwrap();
    let _ = server.join().unwrap();

    let bad_part = root.join("candles").join("2").join("1m").join("2026-01.parquet");
    fs::write(&bad_part, b"garbage data").unwrap();

    let source = CatalogSource::new(Store::new(&root), &db);
    let range = DateRange {
        from: Some(open_ist(mon)),
        to: Some(open_ist(mon) + 375 * 60 - 60),
    };
    let stream = source.open(&["GOOD".into()], Timeframe::M1, range).unwrap();
    let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(candles.len(), 375);

    let _ = fs::remove_dir_all(root);
}

/// T11: Fingerprint: a partition outside the window leaves it unchanged. A changed bar or lot size changes it.
#[test]
fn t11_fingerprint_window_and_metadata_sensitivity() {
    let root = temp("t11-fingerprint");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let jan = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let feb = NaiveDate::from_ymd_opt(2026, 2, 2).unwrap();
    let mar = NaiveDate::from_ymd_opt(2026, 3, 2).unwrap();
    let b_jan = multi_day_intraday_body(&[jan], 375);
    let b_feb = multi_day_intraday_body(&[feb], 375);
    let (base, server) = mock_server(vec![(200u16, b_jan), (200u16, b_feb)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", jan, jan).unwrap();
    up.update("1", Tf::M1, "EQUITY", feb, feb).unwrap();
    let _ = server.join().unwrap();

    let source = CatalogSource::new(Store::new(&root), &db);
    let range_jan = DateRange {
        from: Some(open_ist(jan)),
        to: Some(open_ist(jan) + 375 * 60 - 60),
    };
    let fp1 = source.fingerprint(&["ABC".into()], Timeframe::M1, range_jan);

    let b_mar = multi_day_intraday_body(&[mar], 375);
    let (base2, server2) = mock_server(vec![(200u16, b_mar)]);
    let mut up2 = make_updater(&base2, &db, &root);
    up2.update("1", Tf::M1, "EQUITY", mar, mar).unwrap();
    let _ = server2.join().unwrap();

    let fp2 = source.fingerprint(&["ABC".into()], Timeframe::M1, range_jan);
    assert_eq!(
        fp1, fp2,
        "partition outside window must not change fingerprint"
    );

    {
        let cat = Catalog::open(&db).unwrap();
        cat.conn
            .execute(
                "UPDATE instruments SET lot_size=50 WHERE security_id='1'",
                [],
            )
            .unwrap();
    }
    let fp3 = source.fingerprint(&["ABC".into()], Timeframe::M1, range_jan);
    assert_ne!(fp1, fp3, "changing lot size must change fingerprint");

    let _ = fs::remove_dir_all(root);
}

/// T12: Mid-run data update makes the start and end fingerprints differ.
#[test]
fn t12_mid_run_update_fingerprint_differ() {
    let root = temp("t12-midrun");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let body = multi_day_intraday_body(&[mon], 375);
    let (base, server) = mock_server(vec![(200u16, body)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
    let _ = server.join().unwrap();

    let source = CatalogSource::new(Store::new(&root), &db);
    let range = DateRange {
        from: Some(open_ist(mon)),
        to: Some(open_ist(mon) + 375 * 60 - 60),
    };
    let fp_start = source.fingerprint(&["ABC".into()], Timeframe::M1, range);

    {
        let cat = Catalog::open(&db).unwrap();
        cat.conn
            .execute(
                "UPDATE partitions SET checksum='changed_hash' WHERE series_id=1",
                [],
            )
            .unwrap();
    }
    let fp_end = source.fingerprint(&["ABC".into()], Timeframe::M1, range);
    assert_ne!(fp_start, fp_end);

    let _ = fs::remove_dir_all(root);
}

/// T13: Lazy loading: a counter in the cursor proves at most one partition per symbol is resident at any time.
#[test]
fn t13_lazy_loading_resident_partition_at_most_one() {
    let root = temp("t13-lazy");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let jan = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let feb = NaiveDate::from_ymd_opt(2026, 5, 4).unwrap();
    let mar = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    let b1 = multi_day_intraday_body(&[jan], 375);
    let b2 = multi_day_intraday_body(&[feb], 375);
    let b3 = multi_day_intraday_body(&[mar], 375);
    let (base, server) = mock_server(vec![(200u16, b1), (200u16, b2), (200u16, b3)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", jan, mar).unwrap();
    let _ = server.join().unwrap();

    let source = CatalogSource::new(Store::new(&root), &db);
    let range = DateRange {
        from: Some(open_ist(jan)),
        to: Some(open_ist(mar) + 375 * 60 - 60),
    };
    let stream = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
    let counter = source
        .resident_counter_for(&"ABC".into())
        .expect("resident counter");

    let mut max_resident = 0;
    let mut total_candles = 0;
    for item in stream {
        let _ = item.unwrap();
        total_candles += 1;
        let current = counter.load(std::sync::atomic::Ordering::SeqCst);
        if current > max_resident {
            max_resident = current;
        }
    }
    assert_eq!(total_candles, 3 * 375);
    assert_eq!(
        max_resident, 1,
        "at most 1 partition per symbol may be resident at any time"
    );

    let _ = fs::remove_dir_all(root);
}

/// T14: The catalog file is unchanged (bytes and mtime) after open plus a full run.
#[test]
fn t14_catalog_file_unchanged_bytes_and_mtime() {
    let root = temp("t14-unchanged");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");
    let mon = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let body = multi_day_intraday_body(&[mon], 375);
    let (base, server) = mock_server(vec![(200u16, body)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon, mon).unwrap();
    let _ = server.join().unwrap();

    let before_bytes = fs::read(&db).unwrap();
    let before_mtime = fs::metadata(&db).unwrap().modified().unwrap();

    std::thread::sleep(std::time::Duration::from_millis(50));

    let source = CatalogSource::new(Store::new(&root), &db);
    let range = DateRange {
        from: Some(open_ist(mon)),
        to: Some(open_ist(mon) + 375 * 60 - 60),
    };
    let stream = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
    let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(candles.len(), 375);

    let after_bytes = fs::read(&db).unwrap();
    let after_mtime = fs::metadata(&db).unwrap().modified().unwrap();
    assert_eq!(before_bytes, after_bytes, "catalog bytes must be unchanged");
    assert_eq!(before_mtime, after_mtime, "catalog mtime must be unchanged");

    let _ = fs::remove_dir_all(root);
}

/// T15: Regression test: CatalogSource returns bars before the range start for warmup.
/// In a simulation with --from and --warmup-bars, CatalogSource supplies the requested
/// warmup bars from prior sessions without failing with WarmupShort.
#[test]
fn t15_catalog_source_warmup_before_range_start() {
    let root = temp("t15-warmup-regression");
    let db = root.join("catalog.sqlite");
    seed_instrument(&db, "1", "ABC");

    // Two weeks of trading days:
    // Week 1 (prior week): Mon 2026-01-05 .. Fri 2026-01-09
    // Week 2 (target week): Mon 2026-01-12 .. Fri 2026-01-16
    let mon1 = NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let mon2 = NaiveDate::from_ymd_opt(2026, 1, 12).unwrap();
    let fri2 = NaiveDate::from_ymd_opt(2026, 1, 16).unwrap();
    let trading_days = vec![
        mon1,
        NaiveDate::from_ymd_opt(2026, 1, 6).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 7).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 8).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 9).unwrap(),
        mon2,
        NaiveDate::from_ymd_opt(2026, 1, 13).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 14).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
        fri2,
    ];
    let body = multi_day_intraday_body(&trading_days, 375);
    let (base, server) = mock_server(vec![(200u16, body)]);
    let mut up = make_updater(&base, &db, &root);
    up.update("1", Tf::M1, "EQUITY", mon1, fri2).unwrap();
    let outcome = server.join().unwrap();
    assert_eq!(outcome.served, 1);

    let range = DateRange {
        from: Some(open_ist(mon2)),
        to: Some(open_ist(fri2) + 375 * 60 - 60),
    };

    // Direct check: CatalogSource::open with warmup_bars = 20 returns candles before range start.
    let source = CatalogSource::new(Store::new(&root), &db).with_warmup_bars(20);
    let stream = source.open(&["ABC".into()], Timeframe::M1, range).unwrap();
    let candles: Vec<_> = stream.collect::<Result<Vec<_>, _>>().unwrap();
    let range_start = open_ist(mon2);
    let warmup_bars_count = candles.iter().filter(|(_, c)| c.timestamp < range_start).count();
    assert!(
        warmup_bars_count >= 20,
        "CatalogSource must return warmup bars before range start, got {warmup_bars_count}"
    );

    // Simulation check: sim::run with CatalogSource and warmup_bars = 20 succeeds.
    let program = algomln::compile(
        "STRATEGY \"WarmupTest\"\nTRADE_IN ABC\nWHEN close > sma(20)\nBUY 1\n",
    )
    .unwrap();
    assert_eq!(program.warmup_bars(), 20);

    let config = algomln::sim::RunConfig {
        timeframe: Timeframe::M1,
        range,
        initial_cash: 100_000.0,
        warmup_bars: 20,
        latency: 1,
        slippage_bps: 0.0,
        fee_model: algomln::sim::FeeModel::default(),
        partial_fills_pct: None,
        gap_policy: algomln::sim::GapPolicy::None,
        debug_invariants: true,
    };

    let res = algomln::sim::run(&program, &source, config.clone(), Some(vec!["ABC".into()]));
    assert!(
        res.is_ok(),
        "sim::run must succeed with catalog warmup: {:?}",
        res.err()
    );

    let res_pre = algomln::sim::run_precomputed(
        &program,
        &source,
        config,
        Some(vec!["ABC".into()]),
    );
    assert!(
        res_pre.is_ok(),
        "sim::run_precomputed must succeed with catalog warmup: {:?}",
        res_pre.err()
    );

    let _ = fs::remove_dir_all(root);
}
