use crate::feed::auth::{build_feed_url, parse_and_validate_token, AuthError};
use crate::feed::candle::SessionWindow;
use crate::feed::decoder::{decode, Packet};
use crate::feed::metrics::ConnectionMetrics;
use crate::feed::normalize::{
    ltp_to_paise, Deduper, LttDetectionResult, LttDetector, LttDetectorConfig, Tick,
};
use crate::feed::record::RecordWriter;
use crate::feed::source::{FeedEvent, TickSource};
use crate::feed::subscribe::{batch_subscriptions, InstrumentSubscription, RequestCode, SubscriptionError};
use crate::feed::watchdog::{FeedClock, FeedWatchdog, SystemFeedClock, WatchdogAction};
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::protocol::Message;

pub const DEFAULT_FEED_URL: &str = "wss://api-feed.dhan.co";
pub const DEFAULT_WATCHDOG_TIMEOUT: Duration = Duration::from_secs(40);
pub const DEFAULT_INITIAL_BACKOFF: Duration = Duration::from_secs(1);
pub const DEFAULT_MAX_BACKOFF: Duration = Duration::from_secs(30);
pub const DEFAULT_FEED_CHANNEL_CAPACITY: usize = 10_000;

#[derive(Clone, Debug)]
pub struct FeedConfig {
    pub base_url: String,
    pub token: String,
    pub instruments: Vec<InstrumentSubscription>,
    pub request_code: RequestCode,
    pub record_path: Option<PathBuf>,
    pub planned_session_duration: Option<Duration>,
    pub watchdog_timeout: Duration,
    pub session_window: SessionWindow,
    pub clock: Arc<dyn FeedClock>,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
    pub max_reconnect_attempts: Option<usize>,
    pub ltt_detector_config: LttDetectorConfig,
    pub channel_capacity: usize,
}

impl FeedConfig {
    pub fn new(token: impl Into<String>, instruments: Vec<InstrumentSubscription>) -> Self {
        Self {
            base_url: DEFAULT_FEED_URL.to_string(),
            token: token.into(),
            instruments,
            request_code: RequestCode::Ticker,
            record_path: None,
            planned_session_duration: None,
            watchdog_timeout: DEFAULT_WATCHDOG_TIMEOUT,
            session_window: SessionWindow::for_instrument(true),
            clock: Arc::new(SystemFeedClock),
            initial_backoff: DEFAULT_INITIAL_BACKOFF,
            max_backoff: DEFAULT_MAX_BACKOFF,
            max_reconnect_attempts: None,
            ltt_detector_config: LttDetectorConfig::default(),
            channel_capacity: DEFAULT_FEED_CHANNEL_CAPACITY,
        }
    }

    pub fn with_record_path<P: AsRef<Path>>(mut self, path: P) -> Self {
        self.record_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_clock(mut self, clock: Arc<dyn FeedClock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn with_session_window(mut self, window: SessionWindow) -> Self {
        self.session_window = window;
        self
    }

    pub fn with_is_fno(mut self, is_fno: bool) -> Self {
        self.session_window = SessionWindow::for_instrument(is_fno);
        self
    }

    pub fn with_channel_capacity(mut self, capacity: usize) -> Self {
        self.channel_capacity = capacity;
        self
    }

    pub fn with_request_code(mut self, code: RequestCode) -> Self {
        self.request_code = code;
        self
    }
}

#[derive(Debug, Error)]
pub enum FeedError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error(transparent)]
    Subscription(#[from] SubscriptionError),
    #[error("connection error: {0}")]
    Connection(String),
}

pub struct LiveSource {
    rx: Receiver<FeedEvent>,
    shutdown: Arc<AtomicBool>,
    _worker: Option<std::thread::JoinHandle<()>>,
    metrics: Arc<std::sync::Mutex<ConnectionMetrics>>,
    channel_capacity: usize,
}

impl LiveSource {
    /// Connects to Dhan live market feed in a background thread with an async runtime.
    /// Validates the token against current time and refuses connection if expired.
    pub fn connect(config: FeedConfig) -> Result<Self, FeedError> {
        let now = Utc::now().timestamp();
        let claims = parse_and_validate_token(&config.token, now)?;

        if let Some(planned) = config.planned_session_duration {
            let remaining = claims.time_to_expiry(now);
            if remaining < planned.as_secs() as i64 {
                eprintln!(
                    "WARNING: Dhan token expires in {remaining}s, which is less than the planned session duration of {}s",
                    planned.as_secs()
                );
            }
        }

        // Validate instrument batching before connecting
        let _ = batch_subscriptions(config.request_code, &config.instruments)?;

        let channel_capacity = config.channel_capacity;
        let (tx, rx) = sync_channel::<FeedEvent>(channel_capacity);
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_clone = Arc::clone(&shutdown);

        let metrics = Arc::new(std::sync::Mutex::new(ConnectionMetrics::new()));
        let metrics_clone = Arc::clone(&metrics);

        let handle = std::thread::Builder::new()
            .name("dhan-feed-worker".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = tx.send(FeedEvent::Disconnected { reason: None });
                        eprintln!("failed to create tokio runtime for feed: {e}");
                        return;
                    }
                };

                rt.block_on(run_feed_loop(config, claims.client_id, tx, shutdown_clone, metrics_clone));
            })
            .map_err(|e| FeedError::Connection(e.to_string()))?;

        Ok(Self {
            rx,
            shutdown,
            _worker: Some(handle),
            metrics,
            channel_capacity,
        })
    }

    pub fn stop(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }

    pub fn channel_capacity(&self) -> usize {
        self.channel_capacity
    }

    pub fn metrics(&self) -> ConnectionMetrics {
        self.metrics.lock().unwrap().clone()
    }

    pub fn dropped_ticks(&self) -> u64 {
        self.metrics.lock().unwrap().dropped_ticks
    }
}

impl Drop for LiveSource {
    fn drop(&mut self) {
        self.stop();
    }
}

impl TickSource for LiveSource {
    fn current_time_utc(&self) -> Option<i64> {
        Some(Utc::now().timestamp())
    }

    fn next(&mut self) -> Option<FeedEvent> {
        self.rx.recv().ok()
    }
}

async fn run_feed_loop(
    config: FeedConfig,
    client_id: String,
    tx: SyncSender<FeedEvent>,
    shutdown: Arc<AtomicBool>,
    shared_metrics: Arc<std::sync::Mutex<ConnectionMetrics>>,
) {
    let mut deduper = Deduper::new();
    let mut metrics = ConnectionMetrics::new();
    let mut reconnect_attempt: usize = 0;
    let mut seen_code_counts: std::collections::HashMap<u8, usize> = std::collections::HashMap::new();
    let mut recorder = config
        .record_path
        .as_ref()
        .and_then(|p| RecordWriter::open(p).ok());
    let mut last_drop_log = std::time::Instant::now() - Duration::from_secs(10);
    let mut last_status_line = std::time::Instant::now();

    let subscribe_batches = match batch_subscriptions(config.request_code, &config.instruments) {
        Ok(b) => b,
        Err(_) => return,
    };

    while !shutdown.load(Ordering::SeqCst) {
        let url = build_feed_url(&config.base_url, &config.token, &client_id);

        let connect_res = connect_async(&url).await;
        let (ws_stream, _) = match connect_res {
            Ok(stream) => {
                reconnect_attempt = 0;
                let _ = tx.send(FeedEvent::Connected);
                stream
            }
            Err(e) => {
                metrics.reconnects += 1;
                eprintln!("feed connection failed: {e}");

                // Check fatal auth/refusal vs retry
                reconnect_attempt += 1;
                if let Some(max) = config.max_reconnect_attempts {
                    if reconnect_attempt > max {
                        let _ = tx.send(FeedEvent::Disconnected { reason: None });
                        break;
                    }
                }

                let backoff_secs = (config.initial_backoff.as_secs_f64()
                    * (2.0f64.powi((reconnect_attempt - 1).min(10) as i32)))
                .min(config.max_backoff.as_secs_f64());
                // Add jitter (up to 200ms)
                let delay = Duration::from_secs_f64(backoff_secs + (reconnect_attempt as f64 * 0.05));
                sleep(delay).await;
                continue;
            }
        };

        let (mut write, mut read) = ws_stream.split();

        // Resubscribe all instruments
        let mut subscribe_ok = true;
        for batch in &subscribe_batches {
            if let Ok(json_str) = serde_json::to_string(batch) {
                if let Err(e) = write.send(Message::Text(json_str.into())).await {
                    eprintln!("failed to send subscribe message: {e}");
                    subscribe_ok = false;
                    break;
                }
            }
        }

        if !subscribe_ok {
            continue;
        }

        let mut watchdog = FeedWatchdog::new(
            config.session_window,
            config.watchdog_timeout,
            Arc::clone(&config.clock),
        );
        let mut ltt_detector = LttDetector::new(config.ltt_detector_config.clone());

        loop {
            if shutdown.load(Ordering::SeqCst) {
                let _ = write.close().await;
                let _ = tx.send(FeedEvent::Disconnected { reason: None });
                return;
            }

            tokio::select! {
                msg_opt = read.next() => {
                    let msg = match msg_opt {
                        Some(Ok(m)) => m,
                        Some(Err(e)) => {
                            eprintln!("websocket read error: {e}");
                            break;
                        }
                        None => {
                            eprintln!("websocket stream closed by server");
                            break;
                        }
                    };

                    watchdog.record_activity();

                    match msg {
                        Message::Ping(data) => {
                            let _ = write.send(Message::Pong(data)).await;
                        }
                        Message::Pong(_) => {
                            // Heartbeat ok
                        }
                        Message::Binary(bytes) => {
                            let now_f64 = Utc::now().timestamp_nanos_opt().unwrap_or(0) as f64 / 1_000_000_000.0;
                            let recv_ns = (now_f64 * 1_000_000_000.0).round() as u64;

                            if let Some(r) = recorder.as_mut() {
                                if let Err(e) = r.record_frame(now_f64, &bytes) {
                                    eprintln!("WARNING: failed to write frame to record file: {e}");
                                }
                            }

                            if !bytes.is_empty() {
                                let code = bytes[0];
                                let count = seen_code_counts.entry(code).or_insert(0);
                                if *count < 20 {
                                    *count += 1;
                                    let preview_len = bytes.len().min(16);
                                    let mut hex_preview = String::with_capacity(preview_len * 2);
                                    for b in &bytes[..preview_len] {
                                        use std::fmt::Write;
                                        let _ = write!(hex_preview, "{:02x}", b);
                                    }
                                    eprintln!(
                                        "[RAW FRAME] code: {}, len: {}, hex16: {} (sample {}/20)",
                                        code,
                                        bytes.len(),
                                        hex_preview,
                                        *count
                                    );
                                }
                            }

                            let (header, packet) = match decode(&bytes) {
                                Ok(res) => res,
                                Err(e) => {
                                    metrics.decode_errors += 1;
                                    eprintln!("packet decode error: {e}");
                                    continue;
                                }
                            };

                            metrics.record_packet(header.code, now_f64);

                            match packet {
                                Packet::Ticker { ltp, ltt } => {
                                    let ltt_res = ltt_detector.detect(ltt, now_f64 as i64);
                                    let time_utc = match ltt_res {
                                        LttDetectionResult::Valid { time_utc, convention } => {
                                            metrics.detected_convention = Some(convention);
                                            metrics.last_ltt = Some(time_utc);
                                            metrics.last_recv_minus_ltt = Some(now_f64 - time_utc as f64);
                                            metrics.record_latency(now_f64, time_utc);
                                            time_utc
                                        }
                                        LttDetectionResult::NoTradeTime => {
                                            metrics.zero_ltt_ticks += 1;
                                            continue;
                                        }
                                        LttDetectionResult::Stale { raw_ltt, .. } => {
                                            metrics.stale_ticks += 1;
                                            metrics.last_ltt = Some(raw_ltt);
                                            metrics.last_recv_minus_ltt = Some(now_f64 - raw_ltt as f64);
                                            continue;
                                        }
                                    };
                                    let price_paise = ltp_to_paise(ltp);

                                    if !deduper.accept(header.security_id, time_utc, price_paise) {
                                        metrics.deduped_ticks += 1;
                                        continue;
                                    }

                                    let tick = FeedEvent::Tick(Tick {
                                        security_id: header.security_id,
                                        price_paise,
                                        time_utc,
                                        volume: None,
                                        recv_ns,
                                    });
                                    if !send_event_with_backpressure(
                                        &tx,
                                        tick,
                                        config.channel_capacity,
                                        &mut metrics,
                                        &mut last_drop_log,
                                    ) {
                                        return;
                                    }
                                }
                                Packet::Quote { ltp, ltt, volume, .. } | Packet::Full { ltp, ltt, volume, .. } => {
                                    let ltt_res = ltt_detector.detect(ltt, now_f64 as i64);
                                    let time_utc = match ltt_res {
                                        LttDetectionResult::Valid { time_utc, convention } => {
                                            metrics.detected_convention = Some(convention);
                                            metrics.last_ltt = Some(time_utc);
                                            metrics.last_recv_minus_ltt = Some(now_f64 - time_utc as f64);
                                            metrics.record_latency(now_f64, time_utc);
                                            time_utc
                                        }
                                        LttDetectionResult::NoTradeTime => {
                                            metrics.zero_ltt_ticks += 1;
                                            continue;
                                        }
                                        LttDetectionResult::Stale { raw_ltt, .. } => {
                                            metrics.stale_ticks += 1;
                                            metrics.last_ltt = Some(raw_ltt);
                                            metrics.last_recv_minus_ltt = Some(now_f64 - raw_ltt as f64);
                                            continue;
                                        }
                                    };
                                    let price_paise = ltp_to_paise(ltp);

                                    if !deduper.accept(header.security_id, time_utc, price_paise) {
                                        metrics.deduped_ticks += 1;
                                        continue;
                                    }

                                    let tick = FeedEvent::Tick(Tick {
                                        security_id: header.security_id,
                                        price_paise,
                                        time_utc,
                                        volume: Some(volume as i64),
                                        recv_ns,
                                    });
                                    if !send_event_with_backpressure(
                                        &tx,
                                        tick,
                                        config.channel_capacity,
                                        &mut metrics,
                                        &mut last_drop_log,
                                    ) {
                                        return;
                                    }
                                }
                                Packet::PrevClose { close, oi } => {
                                    let ev = FeedEvent::PrevClose {
                                        security_id: header.security_id,
                                        price_paise: ltp_to_paise(close),
                                        oi,
                                    };
                                    if !send_event_with_backpressure(
                                        &tx,
                                        ev,
                                        config.channel_capacity,
                                        &mut metrics,
                                        &mut last_drop_log,
                                    ) {
                                        return;
                                    }
                                }
                                Packet::Disconnect { reason } => {
                                    eprintln!("received feed disconnect code: {reason}");
                                    let _ = tx.send(FeedEvent::Disconnected { reason: Some(reason) });
                                    if reason == 805 {
                                        // Spec: Code 805 means more than 5 connections exist, so do not retry in a loop
                                        eprintln!("fatal error: Dhan code 805 (connection limit exceeded)");
                                        return;
                                    }
                                    break;
                                }
                                Packet::Unknown { code: _ } => {
                                    metrics.unknown_codes += 1;
                                }
                                Packet::Oi { .. } => {}
                            }
                        }
                        Message::Close(cf) => {
                            eprintln!("websocket closed: {cf:?}");
                            break;
                        }
                        _ => {}
                    }
                }
                _ = sleep(Duration::from_millis(1000)) => {
                    // Update shared metrics
                    if let Ok(mut lock) = shared_metrics.try_lock() {
                        *lock = metrics.clone();
                    }

                    // 1-minute periodic status line
                    if last_status_line.elapsed() >= Duration::from_secs(60) {
                        eprintln!("{}", metrics.status_line());
                        last_status_line = std::time::Instant::now();
                    }

                    // Watchdog check: only arms inside trading window, timeout triggers reconnect
                    if let WatchdogAction::TriggeredTimeout { elapsed_secs, timeout_secs } = watchdog.check_now() {
                        eprintln!("watchdog timeout (no activity for {elapsed_secs}s >= {timeout_secs}s inside trading window); reconnecting...");
                        break;
                    }
                }
            }
        }

        let _ = tx.send(FeedEvent::Disconnected { reason: None });
        metrics.reconnects += 1;
        if let Ok(mut lock) = shared_metrics.try_lock() {
            *lock = metrics.clone();
        }
        reconnect_attempt += 1;
        let delay = (config.initial_backoff * (1 << (reconnect_attempt - 1).min(5))).min(config.max_backoff);
        sleep(delay).await;
    }
}

#[inline]
fn send_event_with_backpressure(
    tx: &SyncSender<FeedEvent>,
    event: FeedEvent,
    capacity: usize,
    metrics: &mut ConnectionMetrics,
    last_drop_log: &mut std::time::Instant,
) -> bool {
    match tx.try_send(event) {
        Ok(()) => true,
        Err(std::sync::mpsc::TrySendError::Full(_)) => {
            metrics.dropped_ticks += 1;
            let now = std::time::Instant::now();
            if now.duration_since(*last_drop_log) >= Duration::from_secs(5) {
                eprintln!(
                    "WARNING: feed channel full ({capacity}/{capacity} queued), dropping ticks due to backpressure (total dropped: {})",
                    metrics.dropped_ticks
                );
                *last_drop_log = now;
            }
            true
        }
        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => false,
    }
}
