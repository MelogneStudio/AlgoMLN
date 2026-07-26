use std::sync::Arc;

use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use tokio::{sync::Mutex, task::JoinHandle};
use tokio_util::sync::CancellationToken;

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

                        if matches!(task_session.status(), SessionStatus::Paused) {
                            continue;
                        }

                        let Some(candle) = assembler.feed(&tick) else {
                            continue;
                        };

                        let candles = {
                            let mut history = task_session.candle_history.lock().await;
                            history.push(candle.clone());
                            history.clone()
                        };

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

    pub fn pause(&self) {
        *self.status.write() = SessionStatus::Paused;
    }

    pub fn resume(&self) {
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
