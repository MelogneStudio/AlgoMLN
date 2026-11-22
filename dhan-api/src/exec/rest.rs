use crate::exec::{
    error::{classify_error, ErrorClass},
    normalize::{parse_rest_order, parse_rupees_to_paise, parse_u64_field, NormalizedOrderUpdate},
    types::{ExecutionMode, ModeConfig, OrderKind, OrderRequest, Side},
};
use reqwest::{
    header::{HeaderMap, HeaderName, HeaderValue},
    Client, StatusCode,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

#[derive(Debug, Error, Clone)]
pub enum ExecRestError {
    #[error("auth error: {0}")]
    Auth(String),
    #[error("throttled (rate limit): {0}")]
    Throttle(String),
    #[error("rejected ({class:?}): {message}")]
    Rejected {
        class: ErrorClass,
        message: String,
        raw_body: Option<String>,
    },
    #[error("unknown outcome ({class:?}): {message}")]
    UnknownOutcome {
        class: ErrorClass,
        message: String,
        raw_body: Option<String>,
    },
    #[error("transport error: {0}")]
    Transport(String),
    #[error("malformed response: {0}")]
    Malformed(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaceOrderResponse {
    pub order_id: String,
    pub order_status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TradeRecord {
    pub exchange_trade_id: String,
    pub traded_quantity: u64,
    pub traded_price_paise: i64,
    pub trade_time: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PositionRecord {
    pub security_id: String,
    pub net_qty: i64,
    pub product_type: String,
    pub buy_qty: u64,
    pub sell_qty: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PnlExitRecord {
    pub profit: Option<f64>,
    pub loss: Option<f64>,
    pub product_type: String,
    pub kill_switch_active: bool,
}

#[derive(Clone)]
pub struct DhanTradingClient {
    mode_config: ModeConfig,
    client_id: String,
    http: Client,
}

impl DhanTradingClient {
    pub fn new(mode_config: ModeConfig, client_id: String) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| Client::new());
        Self {
            mode_config,
            client_id,
            http,
        }
    }

    pub fn mode_config(&self) -> &ModeConfig {
        &self.mode_config
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    fn headers(&self) -> HeaderMap {
        let mut map = HeaderMap::new();
        if let Some(ref token) = self.mode_config.token {
            if let Ok(v) = HeaderValue::from_str(token) {
                map.insert(HeaderName::from_static("access-token"), v);
            }
        }
        if let Ok(v) = HeaderValue::from_str(&self.client_id) {
            map.insert(HeaderName::from_static("client-id"), v);
        }
        map.insert(
            HeaderName::from_static("content-type"),
            HeaderValue::from_static("application/json"),
        );
        map
    }

    /// Startup check per Section 2:
    /// - Live: GET /profile
    /// - Sandbox: GET /orders
    /// - Dry: no-op
    /// A 401 means the token does not belong to that environment.
    pub async fn startup_check(&self) -> Result<(), ExecRestError> {
        match self.mode_config.mode {
            ExecutionMode::Dry => Ok(()),
            ExecutionMode::Sandbox => {
                let url = format!("{}/orders", self.mode_config.base_url);
                let resp = self
                    .http
                    .get(&url)
                    .headers(self.headers())
                    .send()
                    .await
                    .map_err(|e| ExecRestError::Transport(e.to_string()))?;

                if resp.status() == StatusCode::UNAUTHORIZED {
                    return Err(ExecRestError::Auth(
                        "sandbox startup check failed: token unauthorized (401)".into(),
                    ));
                }
                if !resp.status().is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(ExecRestError::Auth(format!(
                        "sandbox startup check failed: {text}"
                    )));
                }
                Ok(())
            }
            ExecutionMode::Live => {
                let url = format!("{}/profile", self.mode_config.base_url);
                let resp = self
                    .http
                    .get(&url)
                    .headers(self.headers())
                    .send()
                    .await
                    .map_err(|e| ExecRestError::Transport(e.to_string()))?;

                if resp.status() == StatusCode::UNAUTHORIZED {
                    return Err(ExecRestError::Auth(
                        "live startup check failed: token unauthorized (401)".into(),
                    ));
                }
                if !resp.status().is_success() {
                    let text = resp.text().await.unwrap_or_default();
                    return Err(ExecRestError::Auth(format!(
                        "live startup check failed: {text}"
                    )));
                }
                Ok(())
            }
        }
    }

    /// Place order: POST /orders
    /// v1 body per Section 7:
    /// dhanClientId, correlationId, transactionType, exchangeSegment NSE_EQ,
    /// productType INTRADAY, orderType MARKET or LIMIT, validity DAY, securityId,
    /// quantity, disclosedQuantity 0, price, triggerPrice 0, afterMarketOrder false.
    pub async fn place_order(
        &self,
        req: &OrderRequest,
        correlation_id: &str,
        tick_size_paise: Option<i64>,
    ) -> Result<PlaceOrderResponse, ExecRestError> {
        if self.mode_config.mode == ExecutionMode::Dry {
            return Ok(PlaceOrderResponse {
                order_id: format!("DRY-{}", req.local_seq),
                order_status: "TRANSIT".to_string(),
            });
        }

        let order_type = match req.kind {
            OrderKind::Market => "MARKET",
            OrderKind::Limit { .. } => "LIMIT",
        };

        // Round limit price to instrument tick size without float noise (half-up)
        let price_rupees = match req.kind {
            OrderKind::Market => 0.0,
            OrderKind::Limit { price_paise } => {
                let tick = tick_size_paise.unwrap_or(5).max(1);
                let rounded_paise = round_to_nearest_tick(price_paise, tick);
                rounded_paise as f64 / 100.0
            }
        };

        let body = serde_json::json!({
            "dhanClientId": self.client_id,
            "correlationId": correlation_id,
            "transactionType": match req.side {
                Side::Buy => "BUY",
                Side::Sell => "SELL",
            },
            "exchangeSegment": "NSE_EQ",
            "productType": "INTRADAY",
            "orderType": order_type,
            "validity": "DAY",
            "securityId": req.security_id,
            "quantity": req.quantity,
            "disclosedQuantity": 0,
            "price": price_rupees,
            "triggerPrice": 0.0,
            "afterMarketOrder": false,
        });

        let url = format!("{}/orders", self.mode_config.base_url);
        let resp = self
            .http
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() || e.is_connect() {
                    ExecRestError::UnknownOutcome {
                        class: ErrorClass::UnknownOutcome,
                        message: e.to_string(),
                        raw_body: None,
                    }
                } else {
                    ExecRestError::Transport(e.to_string())
                }
            })?;

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        let json_err: Option<serde_json::Value> = serde_json::from_str(&body_text).ok();
        let is_failure = json_err
            .as_ref()
            .and_then(|j| j.get("status"))
            .and_then(|v| v.as_str())
            == Some("failure");

        if status.is_success() && !is_failure {
            let json = json_err.unwrap_or(serde_json::Value::Null);
            let order_id = json
                .get("orderId")
                .and_then(|v| v.as_str().or_else(|| v.as_i64().map(|_| "")))
                .map(|s| {
                    if s.is_empty() {
                        json.get("orderId").unwrap().to_string()
                    } else {
                        s.to_string()
                    }
                })
                .unwrap_or_else(|| "UNKNOWN".into());
            let order_status = json
                .get("orderStatus")
                .and_then(|v| v.as_str())
                .unwrap_or("TRANSIT")
                .to_string();

            return Ok(PlaceOrderResponse {
                order_id,
                order_status,
            });
        }

        // Handle error responses using error classification
        let err_code = json_err
            .as_ref()
            .and_then(|j| j.get("errorCode"))
            .and_then(|v| v.as_str());
        let err_type = json_err
            .as_ref()
            .and_then(|j| j.get("errorType"))
            .and_then(|v| v.as_str());
        let err_msg = json_err
            .as_ref()
            .and_then(|j| j.get("errorMessage").or_else(|| j.get("message")))
            .and_then(|v| v.as_str())
            .unwrap_or(&body_text);

        let class = classify_error(Some(status.as_u16()), err_code, err_type, Some(err_msg));

        match class {
            ErrorClass::FatalAuth => Err(ExecRestError::Auth(err_msg.to_string())),
            ErrorClass::Throttle => Err(ExecRestError::Throttle(err_msg.to_string())),
            ErrorClass::UnknownOutcome | ErrorClass::TransientServerError => {
                Err(ExecRestError::UnknownOutcome {
                    class,
                    message: err_msg.to_string(),
                    raw_body: Some(body_text),
                })
            }
            _ => Err(ExecRestError::Rejected {
                class,
                message: err_msg.to_string(),
                raw_body: Some(body_text),
            }),
        }
    }

    /// Cancel order: DELETE /orders/{orderId}
    /// Section 7: treat any 2xx as accepted.
    pub async fn cancel_order(&self, dhan_order_id: &str) -> Result<(), ExecRestError> {
        if self.mode_config.mode == ExecutionMode::Dry {
            return Ok(());
        }

        let url = format!("{}/orders/{}", self.mode_config.base_url, dhan_order_id);
        let resp = self
            .http
            .delete(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() || e.is_connect() {
                    ExecRestError::UnknownOutcome {
                        class: ErrorClass::UnknownOutcome,
                        message: e.to_string(),
                        raw_body: None,
                    }
                } else {
                    ExecRestError::Transport(e.to_string())
                }
            })?;

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();
        let json_opt: Option<serde_json::Value> = serde_json::from_str(&body_text).ok();
        let is_failure = json_opt
            .as_ref()
            .and_then(|j| j.get("status"))
            .and_then(|v| v.as_str())
            == Some("failure");

        if status.is_success() && !is_failure {
            return Ok(());
        }

        let err_code = json_opt
            .as_ref()
            .and_then(|j| j.get("errorCode"))
            .and_then(|v| v.as_str());
        let err_type = json_opt
            .as_ref()
            .and_then(|j| j.get("errorType"))
            .and_then(|v| v.as_str());
        let err_msg = json_opt
            .as_ref()
            .and_then(|j| j.get("errorMessage").or_else(|| j.get("message")))
            .and_then(|v| v.as_str())
            .unwrap_or(&body_text);

        let class = classify_error(Some(status.as_u16()), err_code, err_type, Some(err_msg));

        match class {
            ErrorClass::FatalAuth => Err(ExecRestError::Auth(err_msg.to_string())),
            ErrorClass::Throttle => Err(ExecRestError::Throttle(err_msg.to_string())),
            ErrorClass::UnknownOutcome | ErrorClass::TransientServerError => {
                Err(ExecRestError::UnknownOutcome {
                    class,
                    message: err_msg.to_string(),
                    raw_body: Some(body_text),
                })
            }
            _ => Err(ExecRestError::Rejected {
                class,
                message: err_msg.to_string(),
                raw_body: Some(body_text),
            }),
        }
    }

    /// Read today's orders: GET /orders
    pub async fn get_orders(&self) -> Result<Vec<NormalizedOrderUpdate>, ExecRestError> {
        if self.mode_config.mode == ExecutionMode::Dry {
            return Ok(Vec::new());
        }

        let url = format!("{}/orders", self.mode_config.base_url);
        let resp = self
            .http
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| ExecRestError::Transport(e.to_string()))?;

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            let class = classify_error(Some(status.as_u16()), None, None, Some(&body_text));
            return match class {
                ErrorClass::FatalAuth => Err(ExecRestError::Auth(body_text)),
                ErrorClass::Throttle => Err(ExecRestError::Throttle(body_text)),
                _ => Err(ExecRestError::Transport(format!("HTTP {status}: {body_text}"))),
            };
        }

        let json: serde_json::Value = serde_json::from_str(&body_text)
            .map_err(|e| ExecRestError::Malformed(e.to_string()))?;

        let arr = if let Some(a) = json.as_array() {
            a
        } else if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
            data
        } else {
            return Ok(Vec::new());
        };

        let updates: Vec<NormalizedOrderUpdate> =
            arr.iter().filter_map(|v| parse_rest_order(v).ok()).collect();
        Ok(updates)
    }

    /// Look up single order by correlation ID: GET /orders/external/{correlationId}
    pub async fn get_order_by_correlation(
        &self,
        correlation_id: &str,
    ) -> Result<Option<NormalizedOrderUpdate>, ExecRestError> {
        if self.mode_config.mode == ExecutionMode::Dry {
            return Ok(None);
        }

        let url = format!(
            "{}/orders/external/{}",
            self.mode_config.base_url, correlation_id
        );
        let resp = self
            .http
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| ExecRestError::Transport(e.to_string()))?;

        let status = resp.status();
        if status == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let body_text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            let class = classify_error(Some(status.as_u16()), None, None, Some(&body_text));
            return match class {
                ErrorClass::FatalAuth => Err(ExecRestError::Auth(body_text)),
                ErrorClass::Throttle => Err(ExecRestError::Throttle(body_text)),
                _ => Err(ExecRestError::Transport(format!("HTTP {status}: {body_text}"))),
            };
        }

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body_text) {
            // Check if Dhan returns an error indicating not found or failure
            if json.get("status").and_then(|s| s.as_str()) == Some("failure") {
                let msg = json
                    .get("errorMessage")
                    .or_else(|| json.get("message"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if msg.contains("not found") || msg.contains("no order") {
                    return Ok(None);
                }
                return Err(ExecRestError::Rejected {
                    class: ErrorClass::BusinessRejection,
                    message: msg,
                    raw_body: Some(body_text),
                });
            }
            if json.get("orderId").is_none() {
                return Ok(None);
            }
            if let Ok(norm) = parse_rest_order(&json) {
                return Ok(Some(norm));
            }
        }
        Ok(None)
    }

    /// Read fill details for order: GET /trades/{orderId}
    /// Extracts tradedPrice, tradedQuantity, exchangeTradeId.
    pub async fn get_trades_for_order(
        &self,
        dhan_order_id: &str,
    ) -> Result<Vec<TradeRecord>, ExecRestError> {
        if self.mode_config.mode == ExecutionMode::Dry {
            return Ok(Vec::new());
        }

        let url = format!("{}/trades/{}", self.mode_config.base_url, dhan_order_id);
        let resp = self
            .http
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| ExecRestError::Transport(e.to_string()))?;

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            let class = classify_error(Some(status.as_u16()), None, None, Some(&body_text));
            return match class {
                ErrorClass::FatalAuth => Err(ExecRestError::Auth(body_text)),
                ErrorClass::Throttle => Err(ExecRestError::Throttle(body_text)),
                _ => Err(ExecRestError::Transport(format!("HTTP {status}: {body_text}"))),
            };
        }

        let json: serde_json::Value = serde_json::from_str(&body_text)
            .map_err(|e| ExecRestError::Malformed(e.to_string()))?;

        let arr = if let Some(a) = json.as_array() {
            a
        } else if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
            data
        } else {
            return Ok(Vec::new());
        };

        let mut trades = Vec::new();
        for item in arr {
            if let Some(obj) = item.as_object() {
                let trade_id = obj
                    .get("exchangeTradeId")
                    .or_else(|| obj.get("tradeId"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let qty = obj.get("tradedQuantity").map(parse_u64_field).unwrap_or(0);
                let price_paise = obj
                    .get("tradedPrice")
                    .and_then(parse_rupees_to_paise)
                    .unwrap_or(0);

                let time = obj
                    .get("tradeTime")
                    .or_else(|| obj.get("createTime"))
                    .and_then(|v| v.as_str())
                    .map(String::from);

                if !trade_id.is_empty() && qty > 0 {
                    trades.push(TradeRecord {
                        exchange_trade_id: trade_id,
                        traded_quantity: qty,
                        traded_price_paise: price_paise,
                        trade_time: time,
                    });
                }
            }
        }
        Ok(trades)
    }

    /// Read positions: GET /positions
    pub async fn get_positions(&self) -> Result<Vec<PositionRecord>, ExecRestError> {
        if self.mode_config.mode == ExecutionMode::Dry {
            return Ok(Vec::new());
        }

        let url = format!("{}/positions", self.mode_config.base_url);
        let resp = self
            .http
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| ExecRestError::Transport(e.to_string()))?;

        let status = resp.status();
        let body_text = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            let class = classify_error(Some(status.as_u16()), None, None, Some(&body_text));
            return match class {
                ErrorClass::FatalAuth => Err(ExecRestError::Auth(body_text)),
                ErrorClass::Throttle => Err(ExecRestError::Throttle(body_text)),
                _ => Err(ExecRestError::Transport(format!("HTTP {status}: {body_text}"))),
            };
        }

        let arr = if let Some(a) = json.as_array() {
            a
        } else if let Some(data) = json.get("data").and_then(|d| d.as_array()) {
            data
        } else {
            return Ok(Vec::new());
        };

        let mut positions = Vec::new();
        for item in arr {
            if let Some(obj) = item.as_object() {
                let sec_id = obj
                    .get("securityId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let product = obj
                    .get("productType")
                    .and_then(|v| v.as_str())
                    .unwrap_or("INTRADAY")
                    .to_string();

                let buy_qty = obj.get("buyQty").map(parse_u64_field).unwrap_or(0);
                let sell_qty = obj.get("sellQty").map(parse_u64_field).unwrap_or(0);
                let net_qty = obj
                    .get("netQty")
                    .and_then(|v| v.as_i64())
                    .unwrap_or((buy_qty as i64) - (sell_qty as i64));

                if !sec_id.is_empty() {
                    positions.push(PositionRecord {
                        security_id: sec_id,
                        net_qty,
                        product_type: product,
                        buy_qty,
                        sell_qty,
                    });
                }
            }
        }
        Ok(positions)
    }

    /// Read kill switch status: GET /killswitch
    pub async fn get_kill_switch_status(&self) -> Result<String, ExecRestError> {
        if self.mode_config.mode == ExecutionMode::Dry {
            return Ok("INACTIVE".to_string());
        }

        let url = format!("{}/killswitch", self.mode_config.base_url);
        let resp = self
            .http
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| ExecRestError::Transport(e.to_string()))?;

        let body_text = resp.text().await.unwrap_or_default();
        let json: serde_json::Value = serde_json::from_str(&body_text)
            .map_err(|e| ExecRestError::Malformed(e.to_string()))?;

        let status = json
            .get("killSwitchStatus")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string();
        Ok(status)
    }

    /// Activate kill switch: POST /killswitch?killSwitchStatus=ACTIVATE
    pub async fn activate_kill_switch(&self) -> Result<(), ExecRestError> {
        if self.mode_config.mode == ExecutionMode::Dry {
            return Ok(());
        }

        let url = format!(
            "{}/killswitch?killSwitchStatus=ACTIVATE",
            self.mode_config.base_url
        );
        let resp = self
            .http
            .post(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| ExecRestError::Transport(e.to_string()))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let body_text = resp.text().await.unwrap_or_default();
            Err(ExecRestError::Rejected {
                class: ErrorClass::FatalAccount,
                message: format!("kill switch activation failed: {body_text}"),
                raw_body: Some(body_text),
            })
        }
    }

    /// Configure broker P&L exit backstop: POST /pnlExit
    pub async fn configure_pnl_exit(
        &self,
        loss_rupees: f64,
        profit_rupees: Option<f64>,
        enable_kill_switch: bool,
    ) -> Result<(), ExecRestError> {
        if self.mode_config.mode == ExecutionMode::Dry {
            return Ok(());
        }

        let mut body = serde_json::json!({
            "lossValue": loss_rupees,
            "productType": "INTRADAY",
            "enableKillSwitch": enable_kill_switch,
        });
        if let Some(prof) = profit_rupees {
            body["profitValue"] = serde_json::json!(prof);
        }

        let url = format!("{}/pnlExit", self.mode_config.base_url);
        let resp = self
            .http
            .post(&url)
            .headers(self.headers())
            .json(&body)
            .send()
            .await
            .map_err(|e| ExecRestError::Transport(e.to_string()))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let body_text = resp.text().await.unwrap_or_default();
            Err(ExecRestError::Rejected {
                class: ErrorClass::FatalConfig,
                message: format!("configure pnl exit failed: {body_text}"),
                raw_body: Some(body_text),
            })
        }
    }

    /// Get broker P&L exit config: GET /pnlExit
    pub async fn get_pnl_exit(&self) -> Result<Option<PnlExitRecord>, ExecRestError> {
        if self.mode_config.mode == ExecutionMode::Dry {
            return Ok(None);
        }

        let url = format!("{}/pnlExit", self.mode_config.base_url);
        let resp = self
            .http
            .get(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| ExecRestError::Transport(e.to_string()))?;

        if resp.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }

        let body_text = resp.text().await.unwrap_or_default();
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&body_text) {
            let profit = json.get("profit").and_then(|v| v.as_f64());
            let loss = json.get("loss").and_then(|v| v.as_f64());
            let product_type = json
                .get("productType")
                .and_then(|v| v.as_str())
                .unwrap_or("INTRADAY")
                .to_string();
            let kill_switch_active = json
                .get("enableKillSwitch")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            return Ok(Some(PnlExitRecord {
                profit,
                loss,
                product_type,
                kill_switch_active,
            }));
        }
        Ok(None)
    }

    /// Emergency exit all positions: DELETE /positions
    /// Returns 202 accepted with no body.
    pub async fn delete_all_positions(&self) -> Result<(), ExecRestError> {
        if self.mode_config.mode == ExecutionMode::Dry {
            return Ok(());
        }

        let url = format!("{}/positions", self.mode_config.base_url);
        let resp = self
            .http
            .delete(&url)
            .headers(self.headers())
            .send()
            .await
            .map_err(|e| ExecRestError::Transport(e.to_string()))?;

        if resp.status().is_success() {
            Ok(())
        } else {
            let body_text = resp.text().await.unwrap_or_default();
            Err(ExecRestError::Rejected {
                class: ErrorClass::FatalAccount,
                message: format!("delete all positions failed: {body_text}"),
                raw_body: Some(body_text),
            })
        }
    }
}

/// Round price in paise to the nearest instrument tick size (half-up).
pub fn round_to_nearest_tick(price_paise: i64, tick_paise: i64) -> i64 {
    let tick = tick_paise.max(1);
    ((price_paise + tick / 2) / tick) * tick
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_rounding_half_up() {
        assert_eq!(round_to_nearest_tick(25050, 5), 25050);
        assert_eq!(round_to_nearest_tick(25052, 5), 25050);
        assert_eq!(round_to_nearest_tick(25053, 5), 25055); // 25053 rounds up to 25055
        assert_eq!(round_to_nearest_tick(25054, 5), 25055);
        assert_eq!(round_to_nearest_tick(25055, 5), 25055);
    }
}
