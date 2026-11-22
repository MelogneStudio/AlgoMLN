pub mod error;
pub mod types;

pub use error::{classify_error, ErrorClass};
pub use types::{
    ExecEvent, ExecutionMode, LocalOrderId, ModeConfig, OrderIntent, OrderKind, OrderRequest,
    Side, LIVE_BASE_URL, LIVE_ORDER_UPDATE_WS, SANDBOX_BASE_URL,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mode_resolution_isolation() {
        // Dry mode requires no token and resolves safely
        let dry = ModeConfig::resolve(ExecutionMode::Dry, None).expect("dry mode resolves");
        assert_eq!(dry.mode, ExecutionMode::Dry);
        assert!(!dry.live_trading_acknowledged);

        // Sandbox mode fails if DHAN_SANDBOX_ACCESS_TOKEN is missing
        std::env::remove_var("DHAN_SANDBOX_ACCESS_TOKEN");
        let sb_err = ModeConfig::resolve(ExecutionMode::Sandbox, None);
        assert!(sb_err.is_err());
        assert!(sb_err.unwrap_err().contains("DHAN_SANDBOX_ACCESS_TOKEN"));

        // Live mode fails without LIVE_TRADING=true
        std::env::remove_var("LIVE_TRADING");
        std::env::set_var("DHAN_ACCESS_TOKEN", "mock_live_token");
        let live_err = ModeConfig::resolve(ExecutionMode::Live, None);
        assert!(live_err.is_err());
        assert!(live_err.unwrap_err().contains("LIVE_TRADING=true"));

        // Live mode fails if LIVE_TRADING=true but DHAN_ACCESS_TOKEN missing
        std::env::set_var("LIVE_TRADING", "true");
        std::env::remove_var("DHAN_ACCESS_TOKEN");
        let live_token_err = ModeConfig::resolve(ExecutionMode::Live, None);
        assert!(live_token_err.is_err());
        assert!(live_token_err.unwrap_err().contains("DHAN_ACCESS_TOKEN"));

        // Sandbox succeeds with DHAN_SANDBOX_ACCESS_TOKEN and uses sandbox URL
        std::env::set_var("DHAN_SANDBOX_ACCESS_TOKEN", "sb_secret");
        let sb_ok = ModeConfig::resolve(ExecutionMode::Sandbox, None).unwrap();
        assert_eq!(sb_ok.base_url, SANDBOX_BASE_URL);
        assert_eq!(sb_ok.token.as_deref(), Some("sb_secret"));

        // Live succeeds with both flag and token and uses live URL
        std::env::set_var("DHAN_ACCESS_TOKEN", "live_secret");
        let live_ok = ModeConfig::resolve(ExecutionMode::Live, None).unwrap();
        assert_eq!(live_ok.base_url, LIVE_BASE_URL);
        assert_eq!(live_ok.token.as_deref(), Some("live_secret"));
        assert!(live_ok.live_trading_acknowledged);

        // Test base URL override works for mock servers
        let mock_override = ModeConfig::resolve(
            ExecutionMode::Sandbox,
            Some("http://127.0.0.1:9999/v2".into()),
        )
        .unwrap();
        assert_eq!(mock_override.base_url, "http://127.0.0.1:9999/v2");
    }

    #[test]
    fn test_error_classification_all_specs() {
        // HTTP 401 or DH-901 -> FatalAuth
        assert_eq!(
            classify_error(Some(401), None, None, None),
            ErrorClass::FatalAuth
        );
        assert_eq!(
            classify_error(None, Some("DH-901"), None, None),
            ErrorClass::FatalAuth
        );

        // DH-902 -> FatalConfig
        assert_eq!(
            classify_error(None, Some("DH-902"), None, None),
            ErrorClass::FatalConfig
        );

        // DH-903 -> FatalAccount
        assert_eq!(
            classify_error(None, Some("DH-903"), None, None),
            ErrorClass::FatalAccount
        );

        // HTTP 429, DH-904, RL001, RATE_LIMIT_ERROR -> Throttle
        assert_eq!(
            classify_error(Some(429), None, None, None),
            ErrorClass::Throttle
        );
        assert_eq!(
            classify_error(None, Some("DH-904"), None, None),
            ErrorClass::Throttle
        );
        assert_eq!(
            classify_error(None, Some("RL001"), None, None),
            ErrorClass::Throttle
        );
        assert_eq!(
            classify_error(None, None, Some("RATE_LIMIT_ERROR"), None),
            ErrorClass::Throttle
        );

        // DH-905 -> OurBug
        assert_eq!(
            classify_error(None, Some("DH-905"), None, None),
            ErrorClass::OurBug
        );

        // DH-906 -> BusinessRejection
        assert_eq!(
            classify_error(None, Some("DH-906"), None, None),
            ErrorClass::BusinessRejection
        );

        // DH-907 -> BadRequest
        assert_eq!(
            classify_error(None, Some("DH-907"), None, None),
            ErrorClass::BadRequest
        );

        // DH-908 -> TransientServerError
        assert_eq!(
            classify_error(None, Some("DH-908"), None, None),
            ErrorClass::TransientServerError
        );

        // DH-909 or 5xx -> UnknownOutcome
        assert_eq!(
            classify_error(None, Some("DH-909"), None, None),
            ErrorClass::UnknownOutcome
        );
        assert_eq!(
            classify_error(Some(500), None, None, None),
            ErrorClass::UnknownOutcome
        );
        assert_eq!(
            classify_error(Some(502), None, None, None),
            ErrorClass::UnknownOutcome
        );

        // DH-910 or unseen -> Unclassified
        assert_eq!(
            classify_error(None, Some("DH-910"), None, None),
            ErrorClass::Unclassified
        );
        assert_eq!(
            classify_error(None, Some("XYZ-999"), None, None),
            ErrorClass::Unclassified
        );
    }
}
