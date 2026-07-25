use std::{collections::BTreeMap, sync::Arc};

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
        trade_log::{TradeLog, TradeLogEntry},
    },
    models::{Candle, Order, OrderResult, OrderSide},
    plugin::api::events::EventBus,
    strategy::{
        dsl::StrategyNode,
        execution::DhanBroker,
        logging::{LogEntry, LogEntryKind},
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

                        let logs = {
                            let mut engine = task_session.engine.lock().await;
                            engine.on_candle(&candles).await
                        };

                        task_session.append_executed_orders(&logs, &candle).await;
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
    }

    pub fn status(&self) -> SessionStatus {
        self.status.read().clone()
    }

    async fn append_executed_orders(&self, logs: &[LogEntry], candle: &Candle) {
        let mut submitted_by_rule: BTreeMap<String, Order> = BTreeMap::new();

        for log in logs {
            match &log.kind {
                LogEntryKind::OrderSubmitted { rule_id, order } => {
                    submitted_by_rule.insert(rule_id.clone(), order.clone());
                }
                LogEntryKind::OrderExecuted { rule_id, result } => {
                    let Some(order) = submitted_by_rule.get(rule_id) else {
                        eprintln!(
                            "[live_session] executed order without submitted order for rule {rule_id}"
                        );
                        continue;
                    };

                    if let Err(error) = self
                        .trade_log
                        .append(self.trade_log_entry(log, rule_id, order, result, candle))
                    {
                        eprintln!("[live_session] failed to append trade log: {error}");
                    }
                }
                _ => {}
            }
        }
    }

    fn trade_log_entry(
        &self,
        log: &LogEntry,
        rule_id: &str,
        order: &Order,
        result: &OrderResult,
        candle: &Candle,
    ) -> TradeLogEntry {
        TradeLogEntry {
            id: log.id.clone(),
            timestamp: timestamp_to_rfc3339(result.timestamp.max(log.timestamp)),
            strategy_id: self.strategy_id.clone(),
            strategy_name: self.strategy_name.clone(),
            symbol: order.symbol.clone(),
            side: order_side(order.side).to_string(),
            quantity: i64::from(order.quantity),
            price: order.price.unwrap_or(candle.close),
            order_id: result.order_id.clone(),
            order_status: format!("{:?}", result.status),
            mode: "live".to_string(),
            rule_id: rule_id.to_string(),
            notes: String::new(),
        }
    }
}

fn order_side(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "BUY",
        OrderSide::Sell => "SELL",
    }
}

fn timestamp_to_rfc3339(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(timestamp)
        .unwrap_or_else(Utc::now)
        .to_rfc3339()
}
