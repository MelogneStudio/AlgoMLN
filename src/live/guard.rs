use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Datelike, FixedOffset, Timelike, Weekday};
use parking_lot::RwLock;
use serde::Deserialize;
use uuid::Uuid;

use crate::broker::{
    dhan::rest::DhanClient,
    symbol_map::{Segment, SymbolMap},
    BrokerClient,
};
use crate::live::holidays::NseHolidayCalendar;
use crate::strategy::{dsl::StrategyNode, execution::DhanBroker};

/// IST is UTC+05:30 and has no daylight-saving transitions, so a `FixedOffset`
/// is the right primitive. Using a chrono `Tz` (e.g. `chrono_tz::Asia/Kolkata`)
/// would add a dependency for the sake of a constant offset.
pub const IST_OFFSET_SECONDS: i32 = 5 * 3600 + 30 * 60;

/// Duration of a fresh confirmation token. 90 s gives the user a comfortable
/// window to read the warnings, click the checkbox, and confirm — earlier
/// versions used 30 s, which was too tight against a 3-second countdown and
/// a reader-of-warnings pause.
pub const TOKEN_TTL_SECONDS: u64 = 90;

/// Outcome of `LiveGuard::run_preflight`. Either the strategy is cleared to
/// start (`Ok` with a fresh token) or it needs the user to acknowledge the
/// live-trading warning before confirming (`RequiresAcknowledgment`).
///
/// Both variants carry a token — see the spec's "Ack flow correction"
/// section. `acknowledge_live_trading` is a pure "user checked the box"
/// operation that does not touch the token; the token lifecycle is owned
/// entirely by `request_live_start` and `confirm_live_start`.
pub enum LiveGuardResult {
    Ok { token: PendingLiveToken },
    RequiresAcknowledgment { token: PendingLiveToken },
}

/// A freshly-issued live-start confirmation token. Single use, bound to a
/// specific strategy id, and short-lived. The owning slot lives in
/// `AppState.pending_live_token`; `confirm_live_start` consumes it under the
/// slot lock so concurrent attempts are serialized.
#[derive(Debug, Clone)]
pub struct PendingLiveToken {
    pub token: String,
    pub strategy_id: String,
    pub expires_at: Instant,
}

/// Holds the dependencies the safety gates need and runs them in order.
///
/// The guard is cheap to clone (everything inside is `Arc`-shared) and is
/// safe to call from any tokio task. `run_preflight` is `async` only because
/// gate 2 hits the network via `client.get_positions`; the other gates are
/// pure CPU.
pub struct LiveGuard {
    pub client: Arc<DhanClient>,
    pub symbol_map: Arc<RwLock<SymbolMap>>,
    pub broker: Arc<DhanBroker>,
    pub ack_path: PathBuf,
    pub holiday_calendar: Arc<NseHolidayCalendar>,
}

impl LiveGuard {
    /// Construct a guard with the supplied dependencies.
    pub fn new(
        client: Arc<DhanClient>,
        symbol_map: Arc<RwLock<SymbolMap>>,
        broker: Arc<DhanBroker>,
        ack_path: PathBuf,
        holiday_calendar: Arc<NseHolidayCalendar>,
    ) -> Self {
        Self {
            client,
            symbol_map,
            broker,
            ack_path,
            holiday_calendar,
        }
    }

    /// Run gates 1–8 in order. Returns `Ok(...)` with a fresh 90-second
    /// token on success, or `Err(msg)` describing the first gate that
    /// failed. No token is issued on failure.
    ///
    /// Gate 9 (acknowledgment) is *not* a gate in this method — it merely
    /// chooses between the two `LiveGuardResult` variants. The caller decides
    /// whether to surface the ack modal to the user.
    pub async fn run_preflight(
        &self,
        symbol: &str,
        strategy_id: &str,
        strategy_node: &StrategyNode,
    ) -> Result<LiveGuardResult, String> {
        // Gate 1: paper-default guard. The strategy node must explicitly carry
        // a risk profile marked Live. (We cannot use `strategy_node.mode`
        // because the DSL AST does not store the deployment mode — the mode
        // lives on the registry record. The preflight checks the registry
        // record instead, but we keep this gate documented in the source.)
        //
        // NOTE: The deployed mode is checked at the command layer via the
        // strategy registry, not via `StrategyNode`. The gates defined in
        // the spec are enforced here in the order listed, with the
        // exception of mode which is verified in `commands::live::request_live_start`.

        // Gate 2: broker reachability. A failed network call here is fatal —
        // if we can't talk to Dhan, we can't even fetch the positions used
        // by the daily-loss tracker, so a live session is unsafe.
        self.client.get_positions().await.map_err(|e| {
            format!("broker unreachable; cannot confirm live session is safe: {e}")
        })?;

        // Gate 3 + 4: symbol must be in the map and resolve to NSE equity.
        let entry = {
            let map = self.symbol_map.read();
            map.lookup(symbol)
                .ok_or_else(|| format!("symbol '{symbol}' not found in symbol map"))?
        };
        if entry.segment != Segment::NseEq {
            return Err(format!(
                "Phase 7 only supports NSE equity intraday trading; symbol {symbol} is {:?}",
                entry.segment
            ));
        }

        // Gate 5: market hours. Use the current wall clock in IST. We refuse
        // to start a session outside the 09:15–15:30 window on a trading day.
        let now_ist = chrono::Local::now().with_timezone(&FixedOffset::east_opt(IST_OFFSET_SECONDS).expect("IST offset is in range"));
        if !is_market_open(now_ist, &self.holiday_calendar) {
            return Err(
                "market is closed; live trading is only allowed 09:15–15:30 IST on NSE trading days"
                    .to_string(),
            );
        }

        // Gate 6: at least one risk control declared. We require *any*
        // combination of max_orders, max_positions, or max_daily_loss.
        match &strategy_node.risk {
            None => {
                return Err(
                    "live strategies must declare at least one RISK control \
                     (MAX_ORDERS, MAX_POSITIONS, or MAX_DAILY_LOSS)"
                        .to_string(),
                )
            }
            Some(risk) => {
                if risk.max_orders.is_none()
                    && risk.max_open_positions.is_none()
                    && risk.max_daily_loss_pct.is_none()
                {
                    return Err(
                        "live strategies must declare at least one RISK control \
                         (MAX_ORDERS, MAX_POSITIONS, or MAX_DAILY_LOSS)"
                            .to_string(),
                    );
                }
            }
        }

        // Gate 7: max daily loss specifically. The only hard financial safety
        // net in the engine — non-negotiable. Gate 6 lets any risk control
        // through, but live mode requires THIS one in addition.
        let max_daily_loss_present = strategy_node
            .risk
            .as_ref()
            .and_then(|r| r.max_daily_loss_pct)
            .is_some();
        if !max_daily_loss_present {
            return Err(
                "live strategies must declare RISK MAX_DAILY_LOSS".to_string(),
            );
        }

        // Gate 8: broker's realized-loss cache must be fresh. If the
        // background refresh has been failing, the daily-loss tracker is
        // unreliable and starting a session is unsafe.
        if self.broker.is_stale() {
            return Err(
                "broker realized-loss tracking is stale; check broker connectivity and retry"
                    .to_string(),
            );
        }

        // All gates cleared. Issue a fresh token.
        let token = Self::issue_token(strategy_id);

        // Gate 9: ack file. This is not an abort; it picks the variant.
        if read_ack_file(&self.ack_path) {
            Ok(LiveGuardResult::Ok { token })
        } else {
            Ok(LiveGuardResult::RequiresAcknowledgment { token })
        }
    }

    /// Build a new token for `strategy_id`. The TTL is `TOKEN_TTL_SECONDS`;
    /// a separate `expires_at: Instant` makes the validator trivially
    /// unit-testable without a real clock.
    pub fn issue_token(strategy_id: &str) -> PendingLiveToken {
        PendingLiveToken {
            token: Uuid::new_v4().to_string(),
            strategy_id: strategy_id.to_string(),
            expires_at: Instant::now() + Duration::from_secs(TOKEN_TTL_SECONDS),
        }
    }

    /// Validate a pending token against the supplied `strategy_id` and
    /// `token` string. Returns `Ok(())` only when the token matches the
    /// stored strategy id, the string matches, and the token has not
    /// expired. Any failure returns the same generic error message so the
    /// UI does not leak details ("wrong strategy" vs. "expired" is not a
    /// useful distinction for an attacker probing for token state).
    pub fn validate_token(
        pending: &Option<PendingLiveToken>,
        strategy_id: &str,
        token: &str,
    ) -> Result<(), String> {
        let pending = pending.as_ref().ok_or_else(|| {
            "invalid or expired confirmation token".to_string()
        })?;
        if pending.strategy_id != strategy_id {
            return Err("invalid or expired confirmation token".to_string());
        }
        if pending.token != token {
            return Err("invalid or expired confirmation token".to_string());
        }
        if Instant::now() >= pending.expires_at {
            return Err("invalid or expired confirmation token".to_string());
        }
        Ok(())
    }
}

/// True if the given IST wall-clock instant falls inside NSE equity market
/// hours: Monday–Friday, 09:15–15:30 IST, excluding holidays registered in
/// `holidays`. The boundary is inclusive on the open side (`09:15` is open)
/// and exclusive on the close side (`15:30` is open, `15:30:01` is not).
///
/// Extracted as a pure function so it can be unit-tested with a fake
/// `DateTime<FixedOffset>` without depending on the system clock.
pub fn is_market_open(dt: DateTime<FixedOffset>, holidays: &NseHolidayCalendar) -> bool {
    // Weekend first — the common case for off-hours attempts.
    match dt.weekday() {
        Weekday::Sat | Weekday::Sun => return false,
        _ => {}
    }

    // Holiday calendar.
    let date = dt.date_naive();
    if holidays.is_holiday(date) {
        return false;
    }

    // 09:15:00 inclusive — 15:30:00 inclusive.
    let (hour, minute, second) = (dt.hour(), dt.minute(), dt.second());
    let nanos = dt.nanosecond();
    let after_open = (hour, minute, second, nanos) >= (9, 15, 0, 0);
    let before_close = (hour, minute, second, nanos) <= (15, 30, 0, 0);
    after_open && before_close
}

/// Read the persistent ack file. Returns true only when the file exists,
/// parses as JSON, and has `acknowledged == true`. Any other state (missing
/// file, malformed JSON, parse error, `acknowledged == false`) returns false
/// — the safe default is to require the user to ack again.
fn read_ack_file(path: &Path) -> bool {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(_) => return false,
    };
    match serde_json::from_str::<AckFile>(&raw) {
        Ok(parsed) => parsed.acknowledged,
        Err(_) => false,
    }
}

#[derive(Debug, Deserialize)]
struct AckFile {
    #[serde(default)]
    acknowledged: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::holidays::NseHolidayCalendar;
    use chrono::TimeZone;

    fn ist(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<FixedOffset> {
        let offset = FixedOffset::east_opt(IST_OFFSET_SECONDS).unwrap();
        let nd = chrono::NaiveDate::from_ymd_opt(y, m, d).unwrap();
        let nt = chrono::NaiveTime::from_hms_opt(h, mi, s).unwrap();
        offset
            .from_local_datetime(&nd.and_time(nt))
            .single()
            .expect("valid ist instant")
    }

    // ---- Token tests ----

    #[test]
    fn test_token_expires() {
        let pending = Some(PendingLiveToken {
            token: "abc".to_string(),
            strategy_id: "strat-1".to_string(),
            expires_at: Instant::now() - Duration::from_millis(1),
        });
        assert!(LiveGuard::validate_token(&pending, "strat-1", "abc").is_err());
    }

    #[test]
    fn test_token_wrong_id() {
        let pending = Some(LiveGuard::issue_token("strat-1"));
        assert!(LiveGuard::validate_token(&pending, "strat-2", &pending.as_ref().unwrap().token).is_err());
    }

    #[test]
    fn test_token_wrong_token() {
        let pending = Some(LiveGuard::issue_token("strat-1"));
        assert!(LiveGuard::validate_token(&pending, "strat-1", "wrong").is_err());
    }

    #[test]
    fn test_token_valid() {
        let pending = Some(LiveGuard::issue_token("strat-1"));
        let token = pending.as_ref().unwrap().token.clone();
        assert!(LiveGuard::validate_token(&pending, "strat-1", &token).is_ok());
    }

    #[test]
    fn test_token_none() {
        assert!(LiveGuard::validate_token(&None, "strat-1", "abc").is_err());
    }

    // ---- Market hours tests ----

    fn empty_holidays() -> NseHolidayCalendar {
        NseHolidayCalendar::new()
    }

    #[test]
    fn test_market_open_9_15_monday() {
        assert!(is_market_open(ist(2026, 1, 5, 9, 15, 0), &empty_holidays()));
    }

    #[test]
    fn test_market_open_9_14_monday() {
        assert!(!is_market_open(ist(2026, 1, 5, 9, 14, 59), &empty_holidays()));
    }

    #[test]
    fn test_market_open_15_30_monday() {
        assert!(is_market_open(ist(2026, 1, 5, 15, 30, 0), &empty_holidays()));
    }

    #[test]
    fn test_market_open_15_31_monday() {
        assert!(!is_market_open(ist(2026, 1, 5, 15, 30, 1), &empty_holidays()));
    }

    #[test]
    fn test_market_closed_saturday() {
        // 2026-01-03 is a Saturday.
        assert!(!is_market_open(ist(2026, 1, 3, 10, 0, 0), &empty_holidays()));
    }

    #[test]
    fn test_market_closed_sunday() {
        // 2026-01-04 is a Sunday.
        assert!(!is_market_open(ist(2026, 1, 4, 10, 0, 0), &empty_holidays()));
    }

    #[test]
    fn test_market_closed_holiday() {
        // 2026-01-26 is Republic Day (NSE holiday).
        let holidays = vec![chrono::NaiveDate::from_ymd_opt(2026, 1, 26).unwrap()];
        let cal = NseHolidayCalendar::with_holidays(holidays);
        // 2026-01-26 is a Monday, so without the holiday check it would be open.
        assert!(!is_market_open(ist(2026, 1, 26, 10, 0, 0), &cal));
    }

    // ---- Ack file tests ----

    fn temp_path(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("algomln-guard-test-{}-{}.json", name, std::process::id()));
        dir
    }

    #[test]
    fn test_read_ack_missing_file() {
        let path = temp_path("missing");
        let _ = std::fs::remove_file(&path);
        assert!(!read_ack_file(&path));
    }

    #[test]
    fn test_read_ack_true() {
        let path = temp_path("true");
        std::fs::write(&path, r#"{"acknowledged": true}"#).unwrap();
        assert!(read_ack_file(&path));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_read_ack_false() {
        let path = temp_path("false");
        std::fs::write(&path, r#"{"acknowledged": false}"#).unwrap();
        assert!(!read_ack_file(&path));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_read_ack_malformed() {
        let path = temp_path("malformed");
        std::fs::write(&path, "not json").unwrap();
        assert!(!read_ack_file(&path));
        let _ = std::fs::remove_file(&path);
    }
}