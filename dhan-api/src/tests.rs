use super::*;
use chrono::NaiveDate;
use std::sync::Arc;
use std::time::Duration;

#[test]
fn daily_body_uses_required_fields_only() {
    let _c = DhanClient::with_base(Arc::new(EnvTokenProvider::default()), "http://localhost")
        .unwrap();
    let body = DailyRequest {
        security_id: "1",
        exchange_segment: "NSE_EQ",
        instrument: "EQUITY",
        from_date: "2026-01-01".into(),
        to_date: "2026-01-02".into(),
    };
    let json = serde_json::to_value(body).unwrap();
    assert!(json.get("expiryCode").is_none());
    assert!(json.get("oi").is_none());
}

fn mock(
    responses: Vec<(u16, &'static str)>,
    path: &'static str,
) -> (String, std::thread::JoinHandle<usize>) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let h = std::thread::spawn(move || {
        let mut seen = 0;
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]);
            assert!(req.contains("access-token: secret"));
            assert!(req.contains(path));
            if seen == 0 {
                assert!(req.contains("\"exchangeSegment\":\"NSE_EQ\""));
                assert!(req.contains("\"fromDate\":\"2026-01-01\""));
            }
            seen += 1;
            let phrase = match status {
                200 => "OK",
                400 => "Bad Request",
                401 => "Unauthorized",
                429 => "Too Many Requests",
                500 => "Internal Server Error",
                _ => "Error",
            };
            let response = format!(
                "HTTP/1.1 {status} {phrase}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}{body}",
                body.len(),
                if status == 429 {
                    "Retry-After: 0\r\n\r\n"
                } else {
                    "\r\n"
                }
            );
            stream.write_all(response.as_bytes()).unwrap();
        }
        seen
    });
    (format!("http://{addr}"), h)
}

struct TestToken;
impl TokenProvider for TestToken {
    fn access_token(&self) -> Result<String, ApiError> {
        Ok("secret".into())
    }
}

fn call(base: &str) -> Result<Vec<Candle>, ApiError> {
    DhanClient::with_base(Arc::new(TestToken), base)?.intraday(
        "123",
        "EQUITY",
        "1",
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
    )
}

#[test]
fn mock_daily_uses_exclusive_end_and_normalizes_open_label() {
    let (base, server) = mock(
        vec![(
            200,
            r#"{"open":[1],"high":[2],"low":[1],"close":[2],"volume":[3],"timestamp":[1767225600]}"#,
        )],
        "/charts/historical",
    );
    let out = DhanClient::with_base(Arc::new(TestToken), base)
        .unwrap()
        .daily(
            "123",
            "EQUITY",
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        )
        .unwrap();
    assert_eq!(out[0].ts, 1767225600 + 33300);
    assert_eq!(server.join().unwrap(), 1);
}

#[test]
fn mock_success_and_empty() {
    let (body, server) = mock(
        vec![(
            200,
            r#"{"open":[1],"high":[2],"low":[1],"close":[2],"volume":[10],"timestamp":[1767225600]}"#,
        )],
        "/charts/intraday",
    );
    assert_eq!(call(&body).unwrap().len(), 1);
    assert_eq!(server.join().unwrap(), 1);
    let (body, server) = mock(vec![(200, "")], "/charts/intraday");
    assert!(call(&body).unwrap().is_empty());
    server.join().unwrap();
}

#[test]
fn timestamp_accepts_integral_float_and_rejects_fractional_seconds() {
    let (base, server) = mock(
        vec![(
            200,
            r#"{"open":[1],"high":[2],"low":[1],"close":[2],"volume":[10],"timestamp":[1793159100.0]}"#,
        )],
        "/charts/intraday",
    );
    assert_eq!(call(&base).unwrap()[0].ts, 1793159100);
    server.join().unwrap();
    let (base, server) = mock(
        vec![(
            200,
            r#"{"open":[1],"high":[2],"low":[1],"close":[2],"volume":[10],"timestamp":[1793159100.5]}"#,
        )],
        "/charts/intraday",
    );
    assert!(matches!(call(&base), Err(ApiError::Malformed(_))));
    server.join().unwrap();
}

#[test]
fn mock_429_dh904_backoff_then_success() {
    let (body, server) = mock(
        vec![
            (429, r#"{"errorCode":"DH-904","errorMessage":"slow down"}"#),
            (
                200,
                r#"{"open":[],"high":[],"low":[],"close":[],"volume":[],"timestamp":[]}"#,
            ),
        ],
        "/charts/intraday",
    );
    assert!(call(&body).unwrap().is_empty());
    assert_eq!(server.join().unwrap(), 2);
}

#[test]
fn mock_auth_does_not_retry() {
    let (body, server) = mock(
        vec![(
            401,
            r#"{"errorCode":"DH-902","errorMessage":"invalid token"}"#,
        )],
        "/charts/intraday",
    );
    assert!(matches!(call(&body), Err(ApiError::Auth(_))));
    assert_eq!(server.join().unwrap(), 1);
}

#[test]
fn mock_data_error_and_truncated_json() {
    let (body, server) = mock(
        vec![(
            400,
            r#"{"errorCode":"DH-905","errorMessage":"bad instrument"}"#,
        )],
        "/charts/intraday",
    );
    assert!(matches!(call(&body), Err(ApiError::DataApi { .. })));
    server.join().unwrap();
    let (body, server) = mock(vec![(200, r#"{"open":[1]"#)], "/charts/intraday");
    assert!(matches!(call(&body), Err(ApiError::Malformed(_))));
    server.join().unwrap();
}

#[test]
fn mock_http_200_with_error_body_does_not_swallow() {
    let (body, server) = mock(
        vec![(
            200,
            r#"{"errorCode":"DH-901","errorMessage":"data error on 200"}"#,
        )],
        "/charts/intraday",
    );
    match call(&body) {
        Err(ApiError::DataApi { code, message }) => {
            assert_eq!(code, "DH-901");
            assert_eq!(message, "data error on 200");
        }
        res => panic!("expected DataApi error, got {res:?}"),
    }
    server.join().unwrap();

    let (body, server) = mock(
        vec![(
            200,
            r#"{"status":"failure","remarks":"invalid parameters"}"#,
        )],
        "/charts/intraday",
    );
    match call(&body) {
        Err(ApiError::DataApi { code, message }) => {
            assert_eq!(code, "failure");
            assert_eq!(message, "invalid parameters");
        }
        res => panic!("expected DataApi error, got {res:?}"),
    }
    server.join().unwrap();

    let (body, server) = mock(
        vec![(
            200,
            r#"{"unexpected":"json"}"#,
        )],
        "/charts/intraday",
    );
    assert!(matches!(call(&body), Err(ApiError::Malformed(_))));
    server.join().unwrap();
}

#[test]
fn mock_server_retries_then_succeeds() {
    let (body, server) = mock(
        vec![
            (500, "{}"),
            (
                200,
                r#"{"open":[],"high":[],"low":[],"close":[],"volume":[],"timestamp":[]}"#,
            ),
        ],
        "/charts/intraday",
    );
    assert!(call(&body).unwrap().is_empty());
    assert_eq!(server.join().unwrap(), 2);
}

const DETAILED_HEADER: &str = "EXCH_ID,SEGMENT,SECURITY_ID,ISIN,INSTRUMENT,UNDERLYING_SECURITY_ID,UNDERLYING_SYMBOL,SYMBOL_NAME,DISPLAY_NAME,INSTRUMENT_TYPE,SERIES,LOT_SIZE,SM_EXPIRY_DATE,STRIKE_PRICE,OPTION_TYPE,TICK_SIZE,EXPIRY_FLAG,BRACKET_FLAG,COVER_FLAG,ASM_GSM_FLAG,ASM_GSM_CATEGORY,BUY_SELL_INDICATOR,BUY_CO_MIN_MARGIN_PER,BUY_CO_SL_RANGE_MAX_PERC,BUY_CO_SL_RANGE_MIN_PERC,BUY_BO_MIN_MARGIN_PER,BUY_BO_PROFIT_RANGE_MAX_PERC,BUY_BO_PROFIT_RANGE_MIN_PERC,MTF_LEVERAGE,SM_UPPER_LIMIT,SM_LOWER_LIMIT,SM_FREEZE_QTY,\n";

#[test]
fn scrip_filter_and_tick_conversion() {
    let csv = format!(
        "{DETAILED_HEADER}\
         NSE,E,1,INE1,EQUITY,,abc,ABC LTD,Abc,ES,EQ,1.0,,,,5.0000,,,,,,,,,,,,,,,\n\
         NSE,E,2,INE2,EQUITY,,NO,NO LTD,No,ES,BE,1.0,,,,5.0000,,,,,,,,,,,,,,,\n"
    );
    let x = parse_scrip_master(csv.as_bytes()).unwrap();
    assert_eq!(x.len(), 1);
    assert_eq!(x[0].symbol, "ABC");
    assert_eq!(x[0].tick_size_paise, 5);
    assert_eq!(x[0].instrument, "EQUITY");
    assert!(!x[0].is_fno);
    assert_eq!(x[0].expected_1m_bars_per_session(), 375);
}

#[test]
fn scrip_master_accepts_decimal_whole_lot_sizes_and_filters_to_nse_eq_equity() {
    let csv = format!(
        "{DETAILED_HEADER}\
         NSE,E,1,INE1,EQUITY,,RELIANCE,RELIANCE LTD,Reliance,ES,EQ,1.0,,,,10.0000,,,,,,,,,,,,,,,\n\
         NSE,D,2,NA,FUTSTK,1,RELIANCE,RELIFUT,Reliance Fut,FUT,NA,500.0,,,,10.0000,,,,,,,,,,,,,,,\n"
    );
    let rows = parse_scrip_master(csv.as_bytes()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].lot_size, 1);
    assert_eq!(rows[0].tick_size_paise, 10);
    assert!(rows[0].is_fno);
    assert_eq!(rows[0].expected_1m_bars_per_session(), 360);
}

#[test]
fn scrip_master_rejects_fractional_lots() {
    let csv = format!(
        "{DETAILED_HEADER}\
         NSE,E,1,INE1,EQUITY,,RELIANCE,RELIANCE LTD,Reliance,ES,EQ,1.5,,,,10.0000,,,,,,,,,,,,,,,\n"
    );
    assert!(matches!(
        parse_scrip_master(csv.as_bytes()),
        Err(ApiError::Malformed(_))
    ));
}

#[test]
fn equity_fno_flag_set_by_futstk_or_optstk() {
    let csv = format!(
        "{DETAILED_HEADER}\
         NSE,E,2885,INE002A01018,EQUITY,,RELIANCE,RELIANCE LTD,Reliance,ES,EQ,1.0,,,,10.0000,,,,,,,,,,,,,,,\n\
         NSE,E,1594,INE009A01021,EQUITY,,INFY,INFOSYS LTD,Infosys,ES,EQ,1.0,,,,5.0000,,,,,,,,,,,,,,,\n\
         NSE,E,9999,INE999A01011,EQUITY,,ALOKINDS,ALOK INDUSTRIES LTD,Alok,ES,EQ,1.0,,,,5.0000,,,,,,,,,,,,,,,\n\
         NSE,D,55121,NA,FUTSTK,2885,RELIANCE,RELIFUT,Reliance Fut,FUT,NA,500.0,,,,10.0000,,,,,,,,,,,,,,,\n\
         NSE,D,125951,NA,OPTSTK,1594,INFY,INFYOPT,Infosys Opt,OP,NA,400.0,,,,5.0000,,,,,,,,,,,,,,,\n"
    );
    let rows = parse_scrip_master(csv.as_bytes()).unwrap();
    assert_eq!(rows.len(), 3);
    let reliance = rows.iter().find(|r| r.symbol == "RELIANCE").unwrap();
    let infy = rows.iter().find(|r| r.symbol == "INFY").unwrap();
    let alok = rows.iter().find(|r| r.symbol == "ALOKINDS").unwrap();
    assert!(reliance.is_fno);
    assert!(infy.is_fno);
    assert!(!alok.is_fno);
}

#[test]
fn futidx_and_optidx_do_not_set_fno_on_anything() {
    let csv = format!(
        "{DETAILED_HEADER}\
         NSE,E,100,INE100A01010,EQUITY,,NIFTY,NIFTY LTD,Nifty,ES,EQ,1.0,,,,5.0000,,,,,,,,,,,,,,,\n\
         NSE,D,200,NA,FUTIDX,100,NIFTY,NIFTYFUT,Nifty Fut,FUT,NA,50.0,,,,5.0000,,,,,,,,,,,,,,,\n\
         NSE,D,300,NA,OPTIDX,100,NIFTY,NIFTYOPT,Nifty Opt,OP,NA,50.0,,,,5.0000,,,,,,,,,,,,,,,\n"
    );
    let rows = parse_scrip_master(csv.as_bytes()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "NIFTY");
    assert!(!rows[0].is_fno);
}

#[test]
fn missing_required_column_names_missing_column() {
    let csv =
        "EXCH_ID,SEGMENT,SECURITY_ID,ISIN,INSTRUMENT,UNDERLYING_SYMBOL,LOT_SIZE,TICK_SIZE\n\
               NSE,E,1,INE1,EQUITY,RELIANCE,1.0,10.0000\n";
    let err = parse_scrip_master(csv.as_bytes()).unwrap_err();
    match err {
        ApiError::Malformed(msg) => {
            assert!(
                msg.contains("UNDERLYING_SECURITY_ID")
                    || msg.contains("SERIES")
                    || msg.contains("INSTRUMENT_TYPE"),
                "unexpected error message: {msg}"
            );
        }
        other => panic!("expected ApiError::Malformed, got {other:?}"),
    }
}

#[test]
fn duplicate_nse_symbol_rejected_by_resolver() {
    let rows = vec![
        Scrip {
            security_id: "1".into(),
            symbol: "RELIANCE".into(),
            lot_size: 1,
            tick_size_paise: 10,
            instrument: "EQUITY".into(),
            is_fno: true,
        },
        Scrip {
            security_id: "2".into(),
            symbol: "RELIANCE".into(),
            lot_size: 1,
            tick_size_paise: 10,
            instrument: "EQUITY".into(),
            is_fno: true,
        },
    ];
    let err = ScripResolver::new(rows).unwrap_err();
    assert!(
        matches!(err, ApiError::Malformed(ref msg) if msg.contains("duplicate NSE symbol RELIANCE"))
    );
}

#[test]
#[ignore]
fn real_detailed_scrip_master_integration() {
    let file = std::fs::File::open("sample-data/api-scrip-master-detailed.csv")
        .or_else(|_| std::fs::File::open("../sample-data/api-scrip-master-detailed.csv"))
        .expect("detailed master CSV not found in sample-data");
    let rows = parse_scrip_master(file).expect("failed to parse detailed scrip master");
    assert_eq!(rows.len(), 2391);
    let fno_count = rows.iter().filter(|r| r.is_fno).count();
    assert!(
        (150..=300).contains(&fno_count),
        "fno_count {fno_count} not in 150..=300"
    );
    let reliance = rows
        .iter()
        .find(|r| r.security_id == "2885")
        .expect("reliance not found");
    assert!(reliance.is_fno);
    let alok = rows
        .iter()
        .find(|r| r.symbol == "ALOKINDS")
        .expect("alokinds not found");
    assert!(!alok.is_fno);
}

#[test]
fn limiter_concurrent_wait_enforces_spacing_without_starvation() {
    let limiter = Limiter::provisional(); // cap: 4 -> 250ms spacing
    let start = std::time::Instant::now();
    let handles: Vec<_> = (0..3)
        .map(|_| {
            let lim = limiter.clone();
            std::thread::spawn(move || {
                lim.wait();
                std::time::Instant::now()
            })
        })
        .collect();
    let mut times: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    times.sort();
    let elapsed = times[2].duration_since(start);
    assert!(
        elapsed >= Duration::from_millis(400),
        "expected at least 400ms for 3 requests, got {elapsed:?}"
    );
}

#[test]
fn client_reuses_persistent_runtime_and_http_client() {
    let client = DhanClient::with_base(Arc::new(TestToken), "http://localhost:12345").unwrap();
    let cloned = client.clone_with_base("http://localhost:12346").unwrap();
    assert!(Arc::ptr_eq(&client.runtime, &cloned.runtime));
}
