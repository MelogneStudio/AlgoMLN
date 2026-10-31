use chrono::NaiveDate;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client, StatusCode,
};
use serde::{Deserialize, Serialize};
use std::{
    env,
    sync::{Arc, Mutex},
    time::Duration,
};
use thiserror::Error;

const BASE: &str = "https://api.dhan.co/v2";

pub trait TokenProvider: Send + Sync {
    fn access_token(&self) -> Result<String, ApiError>;
}
#[derive(Clone, Debug)]
pub struct EnvTokenProvider {
    pub variable: String,
}
impl Default for EnvTokenProvider {
    fn default() -> Self {
        Self {
            variable: "DHAN_ACCESS_TOKEN".into(),
        }
    }
}
#[derive(Clone, Debug)]
pub struct KeychainEnvTokenProvider {
    pub service: String,
    pub account: String,
    pub variable: String,
}
impl Default for KeychainEnvTokenProvider {
    fn default() -> Self {
        Self {
            service: "algomln".into(),
            account: "dhan-access-token".into(),
            variable: "DHAN_ACCESS_TOKEN".into(),
        }
    }
}
impl TokenProvider for KeychainEnvTokenProvider {
    fn access_token(&self) -> Result<String, ApiError> {
        if let Ok(entry) = keyring::Entry::new(&self.service, &self.account) {
            if let Ok(token) = entry.get_password() {
                if !token.is_empty() {
                    return Ok(token);
                }
            }
        }
        env::var(&self.variable).map_err(|_| {
            ApiError::Auth("Dhan token missing from OS keychain and DHAN_ACCESS_TOKEN".into())
        })
    }
}
impl TokenProvider for EnvTokenProvider {
    fn access_token(&self) -> Result<String, ApiError> {
        env::var(&self.variable).map_err(|_| {
            ApiError::Auth(
                "missing access token; set DHAN_ACCESS_TOKEN or configure an OS keychain provider"
                    .into(),
            )
        })
    }
}

#[derive(Debug, Error, Clone)]
pub enum ApiError {
    #[error("authentication failed ({0}); refresh the token")]
    Auth(String),
    #[error("rate limited ({code}): {message}")]
    RateLimit {
        code: String,
        message: String,
        retry_after: Option<Duration>,
    },
    #[error("Dhan data API error ({code}): {message}")]
    DataApi { code: String, message: String },
    #[error("server error: {0}")]
    Server(String),
    #[error("malformed response: {0}")]
    Malformed(String),
    #[error("transport error: {0}")]
    Transport(String),
    #[error("severe rate limit (DH-805): {0}")]
    SevereRateLimit(String),
    #[error("Dhan access denied ({0})")]
    Access(String),
    #[error("Dhan bad request ({code}): {message}")]
    BadRequest { code: String, message: String },
    #[error("data unavailable ({0})")]
    DataUnavailable(String),
}
#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    #[serde(default, rename = "errorCode")]
    error_code: String,
    #[serde(default, rename = "errorMessage")]
    error_message: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct IntradayRequest<'a> {
    #[serde(rename = "securityId")]
    pub security_id: &'a str,
    #[serde(rename = "exchangeSegment")]
    pub exchange_segment: &'a str,
    pub instrument: &'a str,
    pub interval: &'a str,
    #[serde(rename = "fromDate")]
    pub from_date: String,
    #[serde(rename = "toDate")]
    pub to_date: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct DailyRequest<'a> {
    #[serde(rename = "securityId")]
    pub security_id: &'a str,
    #[serde(rename = "exchangeSegment")]
    pub exchange_segment: &'a str,
    pub instrument: &'a str,
    #[serde(rename = "fromDate")]
    pub from_date: String,
    #[serde(rename = "toDate")]
    pub to_date: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Candle {
    pub ts: i64,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
}
#[derive(Deserialize)]
struct Columns {
    #[serde(default)]
    open: Vec<f64>,
    #[serde(default)]
    high: Vec<f64>,
    #[serde(default)]
    low: Vec<f64>,
    #[serde(default)]
    close: Vec<f64>,
    #[serde(default)]
    volume: Vec<f64>,
    #[serde(default)]
    timestamp: Vec<serde_json::Number>,
}
impl Columns {
    fn candles(self) -> Result<Vec<Candle>, ApiError> {
        let n = self.timestamp.len();
        if [
            self.open.len(),
            self.high.len(),
            self.low.len(),
            self.close.len(),
            self.volume.len(),
        ]
        .iter()
        .any(|&x| x != n)
        {
            return Err(ApiError::Malformed(
                "column arrays have different lengths".into(),
            ));
        }
        (0..n)
            .map(|i| {
                Ok(Candle {
                    ts: self.timestamp[i]
                        .as_i64()
                        .or_else(|| {
                            self.timestamp[i]
                                .as_f64()
                                .filter(|x| {
                                    x.is_finite()
                                        && x.fract() == 0.0
                                        && *x >= i64::MIN as f64
                                        && *x < -(i64::MIN as f64)
                                })
                                .map(|x| x as i64)
                        })
                        .ok_or_else(|| {
                            ApiError::Malformed(format!(
                                "timestamp at index {i} is not an integer epoch second"
                            ))
                        })?,
                    open: self.open[i],
                    high: self.high[i],
                    low: self.low[i],
                    close: self.close[i],
                    volume: self.volume[i],
                })
            })
            .collect()
    }
}

#[derive(Clone)]
pub struct Limiter {
    inner: Arc<Mutex<Bucket>>,
    cap: usize,
    concurrency: usize,
}
struct Bucket {
    tokens: f64,
    last: std::time::Instant,
    tight_until: Option<std::time::Instant>,
}
impl Limiter {
    // Dhan documents 5 requests/second; operate at 4 with a 250 ms minimum interval.
    pub fn provisional() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Bucket {
                tokens: 1.0,
                last: std::time::Instant::now(),
                tight_until: None,
            })),
            cap: 4,
            concurrency: 2,
        }
    }
    pub fn concurrency(&self) -> usize {
        self.concurrency
    }
    pub fn rate_limit(&self) {
        if let Ok(mut b) = self.inner.lock() {
            b.tokens = 0.0;
            b.tight_until = Some(std::time::Instant::now() + Duration::from_secs(300));
            b.tokens = 0.0;
        }
    }
    pub fn daily_limit_remaining(&self) -> u64 {
        90_000
    }
    pub fn wait(&self) {
        loop {
            let delay = {
                let mut b = self.inner.lock().expect("limiter poisoned");
                let now = std::time::Instant::now();
                let elapsed = now.duration_since(b.last).as_secs_f64();
                b.last = now;
                let cap = if b.tight_until.is_some_and(|t| t > now) {
                    2.0
                } else {
                    self.cap as f64
                };
                if b.tight_until.is_some_and(|t| t <= now) {
                    b.tight_until = None;
                }
                b.tokens = (b.tokens + elapsed * cap).min(cap);
                if b.tokens >= 1.0 {
                    b.tokens -= 1.0;
                    return;
                }
                Duration::from_secs_f64((1.0 - b.tokens) / cap)
            };
            std::thread::sleep(delay);
        }
    }
}

pub struct DhanClient {
    token: Arc<dyn TokenProvider>,
    pub limiter: Limiter,
    base: String,
}
impl DhanClient {
    pub fn new(token: Arc<dyn TokenProvider>) -> Result<Self, ApiError> {
        Self::with_base(token, BASE)
    }
    pub fn with_base(
        token: Arc<dyn TokenProvider>,
        base: impl Into<String>,
    ) -> Result<Self, ApiError> {
        Ok(Self {
            token,
            limiter: Limiter::provisional(),
            base: base.into(),
        })
    }
    pub fn clone_with_base(&self, base: &str) -> Result<Self, ApiError> {
        let mut c = Self::with_base(self.token.clone(), base)?;
        c.limiter = self.limiter.clone();
        Ok(c)
    }
    pub fn intraday(
        &self,
        security_id: &str,
        instrument: &str,
        interval: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Candle>, ApiError> {
        let body = IntradayRequest {
            security_id,
            exchange_segment: "NSE_EQ",
            instrument,
            interval,
            from_date: from.format("%Y-%m-%d").to_string(),
            to_date: to.format("%Y-%m-%d").to_string(),
        };
        let url = format!("{}/charts/intraday", self.base);
        self.post_columns(&url, &body)
    }
    pub fn daily(
        &self,
        security_id: &str,
        instrument: &str,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<Vec<Candle>, ApiError> {
        let next = to
            .succ_opt()
            .ok_or_else(|| ApiError::Malformed("daily end date overflow".into()))?;
        eprintln!("Dhan daily history requested for {from}..={to}; using exclusive toDate {next}");
        let body = DailyRequest {
            security_id,
            exchange_segment: "NSE_EQ",
            instrument,
            from_date: from.format("%Y-%m-%d").to_string(),
            to_date: next.format("%Y-%m-%d").to_string(),
        };
        let url = format!("{}/charts/historical", self.base);
        self.post_columns(&url, &body).map(|mut rows| {
            for row in &mut rows {
                row.ts = row.ts.saturating_add(33_300);
            }
            rows
        })
    }
    fn post_columns<T: Serialize>(&self, url: &str, body: &T) -> Result<Vec<Candle>, ApiError> {
        let token = self.token.access_token()?;
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("access-token"),
            HeaderValue::from_str(&token).map_err(|e| ApiError::Auth(e.to_string()))?,
        );
        let mut attempts = 0;
        loop {
            self.limiter.wait();
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|e| ApiError::Transport(e.to_string()))?;
            let response = runtime.block_on(async {
                let client = Client::builder()
                    .timeout(Duration::from_secs(30))
                    .build()
                    .map_err(|e| ApiError::Transport(e.to_string()))?;
                let response = client
                    .post(url)
                    .headers(headers.clone())
                    .json(body)
                    .send()
                    .await
                    .map_err(|e| ApiError::Transport(e.to_string()))?;
                let status = response.status();
                let retry = response
                    .headers()
                    .get("Retry-After")
                    .and_then(|x| x.to_str().ok())
                    .and_then(|x| x.parse::<u64>().ok())
                    .map(Duration::from_secs);
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|e| ApiError::Transport(e.to_string()))?;
                Ok::<_, ApiError>((status, retry, bytes))
            });
            match response {
                Err(e) => {
                    attempts += 1;
                    if attempts >= 3 {
                        return Err(ApiError::Server(e.to_string()));
                    }
                    std::thread::sleep(Duration::from_millis(100 * 2u64.pow(attempts)));
                }
                Ok((status, retry, bytes)) => {
                    let err: ApiErrorBody =
                        serde_json::from_slice(&bytes).unwrap_or(ApiErrorBody {
                            error_code: String::new(),
                            error_message: String::new(),
                        });
                    if err.error_code == "DH-805" {
                        return Err(ApiError::SevereRateLimit(err.error_message));
                    }
                    if err.error_code == "DH-907" {
                        return Err(ApiError::DataUnavailable(err.error_message));
                    }
                    if err.error_code == "DH-904" || status == StatusCode::TOO_MANY_REQUESTS {
                        self.limiter.rate_limit();
                        if attempts >= 5 {
                            return Err(ApiError::RateLimit {
                                code: err.error_code,
                                message: err.error_message,
                                retry_after: retry,
                            });
                        }
                        attempts += 1;
                        std::thread::sleep(
                            retry.unwrap_or(Duration::from_millis(250 * 2u64.pow(attempts.min(5)))),
                        );
                        continue;
                    }
                    if err.error_code == "DH-905" {
                        return Err(ApiError::DataApi {
                            code: err.error_code,
                            message: err.error_message,
                        });
                    }
                    if !status.is_success() {
                        if status == StatusCode::UNAUTHORIZED || err.error_code.starts_with("DH-10")
                        {
                            return Err(ApiError::Auth(format!(
                                "{} {}",
                                status, err.error_message
                            )));
                        }
                        if status == StatusCode::FORBIDDEN {
                            return Err(ApiError::Access(format!(
                                "HTTP {status} {}",
                                err.error_message
                            )));
                        }
                        if status == StatusCode::BAD_REQUEST {
                            return Err(ApiError::BadRequest {
                                code: err.error_code,
                                message: err.error_message,
                            });
                        }
                        if status.is_server_error() && attempts < 3 {
                            attempts += 1;
                            std::thread::sleep(Duration::from_millis(100 * 2u64.pow(attempts)));
                            continue;
                        }
                        if !err.error_code.is_empty() {
                            return Err(ApiError::DataApi {
                                code: err.error_code,
                                message: err.error_message,
                            });
                        }
                        return Err(ApiError::Server(format!("HTTP {status}")));
                    }
                    if bytes.is_empty() {
                        return Ok(Vec::new());
                    }
                    let columns: Columns = serde_json::from_slice(&bytes)
                        .map_err(|e| ApiError::Malformed(e.to_string()))?;
                    return columns.candles();
                }
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct Scrip {
    pub security_id: String,
    pub symbol: String,
    pub lot_size: u32,
    pub tick_size_paise: i64,
    pub instrument: String,
}
pub fn parse_scrip_master<R: std::io::Read>(reader: R) -> Result<Vec<Scrip>, ApiError> {
    let mut csv = csv::Reader::from_reader(reader);
    let headers = csv
        .headers()
        .map_err(|e| ApiError::Malformed(e.to_string()))?
        .clone();
    let idx = |n: &str| {
        headers
            .iter()
            .position(|x| x == n)
            .ok_or_else(|| ApiError::Malformed(format!("missing scrip column {n}")))
    };
    let (id, sym, lot, tick, exchange, series, kind, name) = (
        idx("SEM_SMST_SECURITY_ID")?,
        idx("SEM_TRADING_SYMBOL")?,
        idx("SEM_LOT_UNITS")?,
        idx("SEM_TICK_SIZE")?,
        idx("SEM_EXM_EXCH_ID")?,
        idx("SEM_SERIES")?,
        idx("SEM_EXCH_INSTRUMENT_TYPE")?,
        idx("SEM_INSTRUMENT_NAME")?,
    );
    let mut out = Vec::new();
    for row in csv.records() {
        let r = row.map_err(|e| ApiError::Malformed(e.to_string()))?;
        if &r[exchange] != "NSE" || &r[series] != "EQ" || &r[kind] != "ES" || &r[name] != "EQUITY" {
            continue;
        }
        let raw: f64 = r[tick]
            .parse()
            .map_err(|_| ApiError::Malformed("invalid SEM_TICK_SIZE".into()))?;
        let paise = raw.round() as i64;
        let lot_raw: f64 = r[lot]
            .parse()
            .map_err(|_| ApiError::Malformed("invalid SEM_LOT_UNITS".into()))?;
        if !lot_raw.is_finite()
            || lot_raw < 1.0
            || lot_raw.fract() != 0.0
            || lot_raw > u32::MAX as f64
        {
            return Err(ApiError::Malformed(
                "SEM_LOT_UNITS must be a positive whole number".into(),
            ));
        }
        if paise <= 0 {
            return Err(ApiError::Malformed(
                "SEM_TICK_SIZE must be positive paise".into(),
            ));
        }
        out.push(Scrip {
            security_id: r[id].into(),
            symbol: r[sym].to_ascii_uppercase(),
            lot_size: lot_raw as u32,
            tick_size_paise: paise,
            instrument: r[name].to_string(),
        });
    }
    Ok(out)
}
pub struct ScripResolver {
    by_symbol: std::collections::HashMap<String, Scrip>,
}
impl ScripResolver {
    pub fn new(rows: Vec<Scrip>) -> Result<Self, ApiError> {
        let mut by_symbol = std::collections::HashMap::new();
        for x in rows {
            if by_symbol.insert(x.symbol.clone(), x.clone()).is_some() {
                return Err(ApiError::Malformed(format!(
                    "duplicate NSE symbol {}",
                    x.symbol
                )));
            }
        }
        Ok(Self { by_symbol })
    }
    pub fn resolve(&self, s: &str) -> Result<&Scrip, ApiError> {
        self.by_symbol
            .get(&s.trim().to_ascii_uppercase())
            .ok_or_else(|| ApiError::DataApi {
                code: "UNKNOWN_SYMBOL".into(),
                message: s.into(),
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
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
                let response=format!("HTTP/1.1 {status} {phrase}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",body.len(),if status==429{"Retry-After: 0\r\n"}else{""},body);
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
    #[test]
    fn scrip_filter_and_tick_conversion() {
        let csv="SEM_SMST_SECURITY_ID,SEM_TRADING_SYMBOL,SEM_LOT_UNITS,SEM_TICK_SIZE,SEM_EXM_EXCH_ID,SEM_SERIES,SEM_EXCH_INSTRUMENT_TYPE,SEM_INSTRUMENT_NAME\n1,abc,1,5.0000,NSE,EQ,ES,EQUITY\n2,NO,1,5,NSE,BE,ES,EQUITY\n";
        let x = parse_scrip_master(csv.as_bytes()).unwrap();
        assert_eq!(x.len(), 1);
        assert_eq!(x[0].symbol, "ABC");
        assert_eq!(x[0].tick_size_paise, 5);
        assert_eq!(x[0].instrument, "EQUITY");
    }
    #[test]
    fn scrip_master_accepts_decimal_whole_lot_sizes_and_filters_to_nse_eq_equity() {
        let csv="SEM_SMST_SECURITY_ID,SEM_TRADING_SYMBOL,SEM_LOT_UNITS,SEM_TICK_SIZE,SEM_EXM_EXCH_ID,SEM_SERIES,SEM_EXCH_INSTRUMENT_TYPE,SEM_INSTRUMENT_NAME\n1,RELIANCE,1.0,10.0000,NSE,EQ,ES,EQUITY\n2,FUT,500.0,10.0000,NSE,,FUTSTK,EQUITY\n";
        let rows = parse_scrip_master(csv.as_bytes()).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].lot_size, 1);
        assert_eq!(rows[0].tick_size_paise, 10);
    }
    #[test]
    fn scrip_master_rejects_fractional_lots() {
        let csv="SEM_SMST_SECURITY_ID,SEM_TRADING_SYMBOL,SEM_LOT_UNITS,SEM_TICK_SIZE,SEM_EXM_EXCH_ID,SEM_SERIES,SEM_EXCH_INSTRUMENT_TYPE,SEM_INSTRUMENT_NAME\n1,RELIANCE,1.5,10.0000,NSE,EQ,ES,EQUITY\n";
        assert!(matches!(
            parse_scrip_master(csv.as_bytes()),
            Err(ApiError::Malformed(_))
        ));
    }
}
