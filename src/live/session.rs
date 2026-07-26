use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

/// Maximum number of completed candles kept in `candle_history`. Older candles
/// are dropped from the front once this is exceeded. The engine only needs a
/// bounded window; retaining the full day is dead weight over long sessions.
const MAX_CANDLE_HISTORY: usize = 5000;

use crate::{
    broker::Timeframe,
    feed::FeedManager,
    live::{
        candle_assembler::CandleAssembler,
        trade_log::TradeLog,
    },
    models::Candle,
    plugin::api::events::EventBus,
    strategy::{
        dsl::StrategyNode,
        execution::{DhanBroker, dhan::SessionContext},
        runtime::{StrategyEngine, StrategyInstance, StrategyStatus as EngineStatus},
    },
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionStatus {
    Starting,
    Running,
    Paused,
    Stopped,
    Failed(String),
}

pub struct LiveSession {
    pub strategy_id: String,
    pub strategy_name: String,
    pub symbol: String,
    pub status: Arc<RwLock<SessionStatus>>,
    pub engine: Arc<Mutex<StrategyEngine>>,
    pub broker: Arc<DhanBroker>,
    pub trade_log: Arc<TradeLog>,
    pub candle_history: Arc<Mutex<Vec<Candle>>>,
    pub start_time: DateTime<Utc>,
    cancel: CancellationToken,
    task: Mutex<Option<JoinHandle<()>>>,
}

impl LiveSession {
    /// Construct and immediately start the tick-listening task.
    pub async fn start(
        strategy_id: String,
        strategy_name: String,
        symbol: String,
        strategy_node: StrategyNode,
        broker: Arc<DhanBroker>,
        feed: Arc<Mutex<FeedManager>>,
        trade_log: Arc<TradeLog>,
        event_bus: Arc<EventBus>,
        initial_candles: Vec<Candle>,
    ) -> Result<Arc<Self>, String> {
        // Write session context onto the broker before the engine starts so
        // every `execute`/`execute_with_meta` call during this session sees
        // the correct strategy metadata in the trade log.
        *broker.session_context.write() = Some(SessionContext {
            strategy_id: strategy_id.clone(),
            strategy_name: strategy_name.clone(),
            mode: "live".to_string(),
        });

        let execution_target = broker.clone();
        let mut engine = StrategyEngine::new(StrategyInstance {
            id: strategy_id.clone(),
            strategy: Arc::new(strategy_node),
            symbol: symbol.clone(),
            timeframe: Timeframe::M1,
            status: EngineStatus::Running,
            execution_target,
        });
        engine.event_bus = Some(event_bus);

        let status = Arc::new(RwLock::new(SessionStatus::Starting));
        let session = Arc::new(Self {
            strategy_id,
            strategy_name,
            symbol: symbol.clone(),
            status: status.clone(),
            engine: Arc::new(Mutex::new(engine)),
            broker,
            trade_log,
            candle_history: Arc::new(Mutex::new(initial_candles)),
            start_time: Utc::now(),
            cancel: CancellationToken::new(),
            task: Mutex::new(None),
        });

        let mut receiver = {
            let mut feed = feed.lock().await;
            feed.subscribe(vec![symbol.clone()])
                .await
                .map_err(|error| error.to_string())?;
            feed.subscribe_ticks()
        };

        *status.write() = SessionStatus::Running;

        let task_session = session.clone();
        let task = tokio::spawn(async move {
            let mut assembler = CandleAssembler::new(symbol);

            loop {
                tokio::select! {
                    _ = task_session.cancel.cancelled() => break,
                    tick = receiver.recv() => {
                        let tick = match tick {
                            Ok(tick) => tick,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                                eprintln!("[live_session] tick receiver lagged by {skipped} message(s)");
                                continue;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        };

                        if tick.symbol != task_session.symbol {
                            continue;
                        }

                        // If the realized-loss cache has gone stale, pause immediately
                        // rather than running the engine with an unreliable safety metric.
                        // Only transition if currently Running (don't stomp Failed/Stopped).
                        if task_session.broker.is_stale() {
                            let should_pause = {
                                let s = task_session.status.read();
                                *s == SessionStatus::Running
                            };
                            if should_pause {
                                task_session.broker.set_paused_for_entries(true);
                                *task_session.status.write() = SessionStatus::Paused;
                                eprintln!(
                                    "[live_session] realized-loss cache stale; \
                                     session paused until cache recovers"
                                );
                            }
                        }

                        let Some(candle) = assembler.feed(&tick) else {
                            continue;
                        };

                        // Append the new candle and clone the history while the lock is held;
                        // the guard is dropped at the end of this block — before any .await.
                        let candles = {
                            let mut history = task_session.candle_history.lock().await;
                            history.push(candle.clone());
                            // Keep only the most recent MAX_CANDLE_HISTORY candles; older
                            // candles are dead weight once the indicator windows have slid past them.
                            if history.len() > MAX_CANDLE_HISTORY {
                                let excess = history.len() - MAX_CANDLE_HISTORY;
                                history.drain(0..excess);
                            }
                            history.clone()
                        }; // history guard dropped here — no lock held across the awaits below.

                        // NOTE: the engine lock (tokio::sync::Mutex) is held across
                        // on_candle's await, which includes the broker HTTP call. This is
                        // safe (no UB, Send) but serialises candles if multiple tasks ever
                        // compete for the lock. Phase 7 is single-session so this is
                        // acceptable; Phase 8 should return OrderIntents from on_candle and
                        // execute them outside the lock.
                        let mut engine = task_session.engine.lock().await;
                        engine.on_candle(&candles).await;
                    }
                }
            }

            let mut status = task_session.status.write();
            if !matches!(*status, SessionStatus::Failed(_)) {
                *status = SessionStatus::Stopped;
            }
        });

        *session.task.lock().await = Some(task);
        Ok(session)
    }

    /// Pause the session. Candle data continues to accumulate and the engine
    /// still runs `on_candle` every minute — stop-loss, take-profit, and
    /// risk-breach orders are **not** suppressed. Only new *entry* (BUY) orders
    /// are blocked, via the broker's `paused_for_entries` flag.
    pub fn pause(&self) {
        self.broker.set_paused_for_entries(true);
        *self.status.write() = SessionStatus::Paused;
    }

    pub fn resume(&self) {
        self.broker.set_paused_for_entries(false);
        *self.status.write() = SessionStatus::Running;
    }

    pub async fn stop(&self) {
        self.cancel.cancel();
        if let Some(task) = self.task.lock().await.take() {
            if let Err(error) = task.await {
                eprintln!("[live_session] task join failed: {error}");
            }
        }
        // Clear session context so the broker no longer attaches this
        // strategy's metadata to any subsequent (unexpected) execute calls.
        *self.broker.session_context.write() = None;
    }

    pub fn status(&self) -> SessionStatus {
        self.status.read().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Candle;

    fn make_candle(ts: i64) -> Candle {
        Candle { timestamp: ts, open: 100.0, high: 101.0, low: 99.0, close: 100.5, volume: 1000.0 }
    }

    #[test]
    fn test_candle_history_capped() {
        let mut history: Vec<Candle> = (0..(MAX_CANDLE_HISTORY as i64 + 100))
            .map(make_candle)
            .collect();
        // Apply the same logic as the tick loop.
        if history.len() > MAX_CANDLE_HISTORY {
            let excess = history.len() - MAX_CANDLE_HISTORY;
            history.drain(0..excess);
        }
        assert_eq!(history.len(), MAX_CANDLE_HISTORY);
        // Oldest 100 candles (timestamps 0..100) were evicted; first remaining is 100.
        assert_eq!(history[0].timestamp, 100);
        assert_eq!(history[MAX_CANDLE_HISTORY - 1].timestamp, MAX_CANDLE_HISTORY as i64 + 99);
    }

    /// Verify that a `RecvError::Lagged` on the broadcast channel is recoverable:
    /// the tick loop matches it to `continue`, not `break`, so the session stays alive.
    /// This test demonstrates the channel contract that underpins that logic.
    #[tokio::test]
    async fn test_lag_error_does_not_break_loop() {
        // Channel with capacity 1 — send two items before reading to force Lagged.
        let (tx, mut rx) = tokio::sync::broadcast::channel::<i32>(1);
        let _ = tx.send(1);
        let _ = tx.send(2); // overflows the buffer; receiver will see Lagged on next recv

        let first = rx.recv().await;
        assert!(
            matches!(first, Err(tokio::sync::broadcast::error::RecvError::Lagged(_))),
            "expected Lagged when receiver falls behind"
        );

        // After Lagged the receiver is still valid — the loop `continue`s and
        // picks up the next available tick without breaking the session.
        let _ = tx.send(3);
        assert!(
            rx.recv().await.is_ok(),
            "receiver must still work after a Lagged error"
        );
    }
}
