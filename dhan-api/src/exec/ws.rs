use crate::exec::normalize::{parse_ws_order, NormalizedOrderUpdate};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::{
    sync::{mpsc, watch},
    time::sleep,
};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};

pub const DEFAULT_WS_ORDER_UPDATE_URL: &str = "wss://api-order-update.dhan.co";
pub const DEFAULT_STALENESS_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub enum WsConnectionEvent {
    Connected,
    Reconnected,
    Disconnected(String),
}

pub struct OrderUpdateWsConfig {
    pub url: String,
    pub client_id: String,
    pub token: String,
    pub staleness_timeout: Duration,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for OrderUpdateWsConfig {
    fn default() -> Self {
        Self {
            url: DEFAULT_WS_ORDER_UPDATE_URL.into(),
            client_id: String::new(),
            token: String::new(),
            staleness_timeout: DEFAULT_STALENESS_TIMEOUT,
            initial_backoff: Duration::from_secs(1),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// Spawns a background task that maintains a persistent WebSocket connection
/// to the live Dhan order update service, parses frames into NormalizedOrderUpdate,
/// and detects staleness/drops with exponential reconnect backoff.
pub fn spawn_order_update_ws(
    config: OrderUpdateWsConfig,
    update_tx: mpsc::Sender<NormalizedOrderUpdate>,
    conn_event_tx: mpsc::Sender<WsConnectionEvent>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut backoff = config.initial_backoff;
        let mut is_reconnect = false;

        loop {
            if *shutdown_rx.borrow() {
                break;
            }

            // Attempt connection
            match connect_async(&config.url).await {
                Ok((mut ws_stream, _)) => {
                    // Reset backoff upon successful connection
                    backoff = config.initial_backoff;

                    // Send authentication handshake frame
                    let auth_msg = serde_json::json!({
                        "LoginReq": {
                            "MsgCode": 42,
                            "ClientId": config.client_id,
                            "Token": config.token,
                        },
                        "UserType": "SELF"
                    });

                    if let Err(e) = ws_stream.send(Message::Text(auth_msg.to_string())).await {
                        let _ = conn_event_tx
                            .send(WsConnectionEvent::Disconnected(format!(
                                "failed sending auth frame: {e}"
                            )))
                            .await;
                        sleep(backoff).await;
                        backoff = (backoff * 2).min(config.max_backoff);
                        continue;
                    }

                    // Notify connection status
                    let ev = if is_reconnect {
                        WsConnectionEvent::Reconnected
                    } else {
                        is_reconnect = true;
                        WsConnectionEvent::Connected
                    };
                    let _ = conn_event_tx.send(ev).await;

                    // Message loop with staleness watchdog
                    loop {
                        tokio::select! {
                            _ = shutdown_rx.changed() => {
                                if *shutdown_rx.borrow() {
                                    return;
                                }
                            }
                            msg_opt = tokio::time::timeout(config.staleness_timeout, ws_stream.next()) => {
                                match msg_opt {
                                    Ok(Some(Ok(Message::Text(text)))) => {
                                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                                            if let Ok(norm) = parse_ws_order(&json) {
                                                let _ = update_tx.send(norm).await;
                                            }
                                        }
                                    }
                                    Ok(Some(Ok(Message::Ping(p)))) => {
                                        let _ = ws_stream.send(Message::Pong(p)).await;
                                    }
                                    Ok(Some(Ok(Message::Close(_)))) | Ok(Some(Err(_))) | Ok(None) => {
                                        let _ = conn_event_tx.send(WsConnectionEvent::Disconnected("stream closed".into())).await;
                                        break;
                                    }
                                    Err(_) => {
                                        // Staleness watchdog timeout
                                        let _ = conn_event_tx.send(WsConnectionEvent::Disconnected("staleness watchdog fired".into())).await;
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    let _ = conn_event_tx
                        .send(WsConnectionEvent::Disconnected(e.to_string()))
                        .await;
                }
            }

            // Exponential backoff before reconnect
            sleep(backoff).await;
            backoff = (backoff * 2).min(config.max_backoff);
        }
    })
}
