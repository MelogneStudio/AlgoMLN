use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::Serialize;
use tokio::sync::broadcast;

use crate::plugin::types::{NotificationKind, PluginResult};

use super::{UiApi, UiPanel};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UiMessage {
    PanelRegistered { id: String, title: String },
    Notification { msg: String, kind: String },
    PanelData {
        panel_id: String,
        data: serde_json::Value,
    },
}

pub struct TauriUiApi {
    sender: broadcast::Sender<UiMessage>,
    panels: Arc<RwLock<Vec<(String, String)>>>,
    panel_data: Arc<RwLock<HashMap<String, serde_json::Value>>>,
}

impl TauriUiApi {
    pub fn new() -> (Arc<Self>, broadcast::Receiver<UiMessage>) {
        let (sender, receiver) = broadcast::channel(256);
        let api = Arc::new(Self {
            sender,
            panels: Arc::new(RwLock::new(Vec::new())),
            panel_data: Arc::new(RwLock::new(HashMap::new())),
        });
        (api, receiver)
    }

    pub fn receiver(&self) -> broadcast::Receiver<UiMessage> {
        self.sender.subscribe()
    }
}

#[async_trait::async_trait]
impl UiApi for TauriUiApi {
    fn register_panel(&self, panel: UiPanel) -> PluginResult<()> {
        let UiPanel {
            id,
            title,
            route: _,
        } = panel;
        self.panels.write().push((id.clone(), title.clone()));
        let _ = self.sender.send(UiMessage::PanelRegistered { id, title });
        Ok(())
    }

    fn notify(&self, kind: NotificationKind, message: &str) -> PluginResult<()> {
        let _ = self.sender.send(UiMessage::Notification {
            kind: kind.to_string(),
            msg: message.to_string(),
        });
        Ok(())
    }
}

impl TauriUiApi {
    pub fn emit_panel_data(&self, panel_id: String, data: serde_json::Value) -> PluginResult<()> {
        self.panel_data
            .write()
            .insert(panel_id.clone(), data.clone());
        let _ = self.sender.send(UiMessage::PanelData { panel_id, data });
        Ok(())
    }

    pub fn list_panels(&self) -> Vec<(String, String)> {
        self.panels.read().clone()
    }
}
