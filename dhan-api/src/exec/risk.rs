use crate::exec::types::{OrderIntent, OrderKind, OrderRequest};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct RiskRailsConfig {
    pub max_qty_per_order: Option<u64>,
    pub max_value_per_order_paise: Option<i64>,
    pub max_orders_per_minute: Option<u32>,
    pub broker_kill_switch_on_breach: bool,
    pub allow_emergency_delete_all_positions: bool,
}

impl Default for RiskRailsConfig {
    fn default() -> Self {
        Self {
            max_qty_per_order: Some(10_000),
            max_value_per_order_paise: Some(25_000_000), // 2.5 lakh rupees
            max_orders_per_minute: Some(60),
            broker_kill_switch_on_breach: false,
            allow_emergency_delete_all_positions: false,
        }
    }
}

pub struct RiskController {
    config: RiskRailsConfig,
    entries_locked: bool,
    lock_reason: Option<String>,
    recent_order_times: Vec<Instant>,
}

impl RiskController {
    pub fn new(config: RiskRailsConfig) -> Self {
        Self {
            config,
            entries_locked: false,
            lock_reason: None,
            recent_order_times: Vec::new(),
        }
    }

    pub fn is_entries_locked(&self) -> bool {
        self.entries_locked
    }

    pub fn lock_reason(&self) -> Option<&str> {
        self.lock_reason.as_deref()
    }

    pub fn lock_entries(&mut self, reason: &str) {
        self.entries_locked = true;
        self.lock_reason = Some(reason.to_string());
    }

    pub fn unlock_entries(&mut self) {
        self.entries_locked = false;
        self.lock_reason = None;
    }

    /// Check order against risk limits (Section 10):
    /// - Local entry lock (never blocks exits)
    /// - Max quantity per order
    /// - Max value per order
    /// - Max orders per minute rate limit
    pub fn validate_order(&mut self, req: &OrderRequest) -> Result<(), &'static str> {
        // Exits are NEVER blocked by entry lock
        if req.intent == OrderIntent::Exit {
            return Ok(());
        }

        if self.entries_locked {
            return Err("entries locked: trading halted or entry safety lock engaged");
        }

        if let Some(max_qty) = self.config.max_qty_per_order {
            if req.quantity > max_qty {
                return Err("order rejected: quantity exceeds max_qty_per_order");
            }
        }

        if let Some(max_val) = self.config.max_value_per_order_paise {
            let price_paise = match req.kind {
                OrderKind::Limit { price_paise } => price_paise,
                OrderKind::Market => 0, // For market, strategy sizing applies
            };
            if price_paise > 0 {
                let notional = (req.quantity as i64).saturating_mul(price_paise);
                if notional > max_val {
                    return Err("order rejected: notional value exceeds max_value_per_order");
                }
            }
        }

        // Sliding 1-minute window check
        let now = Instant::now();
        self.recent_order_times
            .retain(|t| now.duration_since(*t).as_secs() < 60);

        if let Some(max_rate) = self.config.max_orders_per_minute {
            if self.recent_order_times.len() >= max_rate as usize {
                return Err("order rejected: orders per minute cap exceeded");
            }
        }

        self.recent_order_times.push(now);
        Ok(())
    }

    pub fn config(&self) -> &RiskRailsConfig {
        &self.config
    }
}

/// Inspect JWT token claims to determine expiration.
/// Section 10: Read exp, warn before it ends. Expiry mid-session locks entries and alerts.
pub fn inspect_jwt_expiry(token: &str) -> Result<i64, &'static str> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return Err("malformed jwt: expected at least two segments");
    }

    let payload_b64 = parts[1];
    // Add padding if missing
    let padded = match payload_b64.len() % 4 {
        2 => format!("{}==", payload_b64),
        3 => format!("{}=", payload_b64),
        _ => payload_b64.to_string(),
    };

    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(&padded)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(&padded))
        .map_err(|_| "failed decoding jwt payload base64")?;

    let json: serde_json::Value = serde_json::from_slice(&decoded)
        .map_err(|_| "failed parsing jwt payload json")?;

    let exp = json
        .get("exp")
        .and_then(|v| v.as_i64())
        .ok_or("jwt payload missing 'exp' field")?;

    Ok(exp)
}
