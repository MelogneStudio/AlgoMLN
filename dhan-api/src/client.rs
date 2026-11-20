use crate::{
    error::{ApiError, ApiErrorBody},
    limiter::Limiter,
    token::TokenProvider,
};
use chrono::NaiveDate;
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client, StatusCode,
};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, time::Duration};

pub const BASE: &str = "https://api.dhan.co/v2";

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
    timestamp: Option<Vec<serde_json::Number>>,
}

impl Columns {
    fn candles(self) -> Result<Vec<Candle>, ApiError> {
        let Some(timestamp) = self.timestamp else {
            return Err(ApiError::Malformed(
                "response missing timestamp column".into(),
            ));
        };
        let n = timestamp.len();
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
                    ts: timestamp[i]
                        .as_i64()
                        .or_else(|| {
                            timestamp[i]
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
                            ApiError::Malformed("timestamp is not a whole integer".into())
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

pub struct DhanClient {
    token: Arc<dyn TokenProvider>,
    pub limiter: Limiter,
    base: String,
    http: Client,
    pub(crate) runtime: Arc<tokio::runtime::Runtime>,
}

impl DhanClient {
    pub fn new(token: Arc<dyn TokenProvider>) -> Result<Self, ApiError> {
        Self::with_base(token, BASE)
    }

    pub fn with_base(
        token: Arc<dyn TokenProvider>,
        base: impl Into<String>,
    ) -> Result<Self, ApiError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        let http = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| ApiError::Transport(e.to_string()))?;
        Ok(Self {
            token,
            limiter: Limiter::provisional(),
            base: base.into(),
            http,
            runtime: Arc::new(runtime),
        })
    }

    pub fn clone_with_base(&self, base: &str) -> Result<Self, ApiError> {
        Ok(Self {
            token: self.token.clone(),
            limiter: self.limiter.clone(),
            base: base.into(),
            http: self.http.clone(),
            runtime: self.runtime.clone(),
        })
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
            HeaderValue::from_str(token.trim()).map_err(|e| ApiError::Auth(e.to_string()))?,
        );
        let mut attempts = 0;
        loop {
            self.limiter.wait();
            let response = self.runtime.block_on(async {
                let response = self
                    .http
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
                        return Err(e);
                    }
                    std::thread::sleep(Duration::from_millis(100 * 2u64.pow(attempts)));
                }
                Ok((status, retry, bytes)) => {
                    let err: ApiErrorBody =
                        serde_json::from_slice(&bytes).unwrap_or(ApiErrorBody {
                            error_code: String::new(),
                            error_message: String::new(),
                            status: None,
                            remarks: None,
                        });
                    let has_explicit_error = !err.error_code.is_empty()
                        || err
                            .status
                            .as_deref()
                            .map(|s| {
                                s.eq_ignore_ascii_case("failure") || s.eq_ignore_ascii_case("error")
                            })
                            .unwrap_or(false);

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
                    if status == StatusCode::UNAUTHORIZED || err.error_code.starts_with("DH-10") {
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
                    if !status.is_success() {
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
                    if has_explicit_error {
                        let code = if !err.error_code.is_empty() {
                            err.error_code
                        } else {
                            err.status.unwrap_or_else(|| "FAILURE".into())
                        };
                        let message = if !err.error_message.is_empty() {
                            err.error_message
                        } else {
                            err.remarks.unwrap_or_default()
                        };
                        return Err(ApiError::DataApi { code, message });
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
