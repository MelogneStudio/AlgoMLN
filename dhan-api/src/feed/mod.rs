pub mod auth;
pub mod candle;
pub mod connection;
pub mod decoder;
pub mod metrics;
pub mod normalize;
pub mod record;
pub mod replay;
pub mod source;
pub mod subscribe;
pub mod watchdog;

pub use auth::{build_feed_url, parse_and_validate_token, AuthError, TokenClaims};
pub use candle::{CandleBuilder, CandleBuilderConfig, ClosedCandle, SessionWindow};
pub use connection::{FeedConfig, FeedError, LiveSource};
pub use decoder::{decode, DecodeError, DepthLevel, Header, Packet};
pub use metrics::ConnectionMetrics;
pub use normalize::{
    ltp_to_paise, ltt_to_utc_seconds, Deduper, LttConvention, LttDetectionResult, LttDetector,
    LttDetectorConfig, Tick, IST_OFFSET_SECONDS,
};
pub use record::RecordWriter;
pub use replay::ReplaySource;
pub use source::{FeedEvent, TickSource};
pub use subscribe::{
    batch_subscriptions, InstrumentSubscription, RequestCode, SubscribeMessage,
    SubscriptionError,
};
pub use watchdog::{
    FeedClock, FeedWatchdog, ManualFeedClock, SystemFeedClock, WatchdogAction,
};
