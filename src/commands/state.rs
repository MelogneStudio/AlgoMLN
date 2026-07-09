use std::sync::Arc;

use tokio::sync::broadcast;

use crate::commands::data::DataState;
use crate::commands::registry::StrategyRegistry;
use crate::plugin::api::ui::UiMessage;
use crate::plugin::registry::PluginRegistry;

/// Tauri-managed application state. Owned by the binary crate but declared
/// here so the `commands::*` modules can use it as a `tauri::State`
/// parameter without depending on `crate::AppState` (which is not visible
/// from the library crate).
pub struct AppState {
    pub data: DataState,
    pub strategies: Arc<StrategyRegistry>,
    pub plugin_registry: Arc<PluginRegistry>,
    pub ui_receiver: broadcast::Receiver<UiMessage>,
}
