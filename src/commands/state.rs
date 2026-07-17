use std::sync::Arc;

use tokio::sync::broadcast;

use crate::broker::symbol_map::SymbolMap;
use crate::commands::data::DataState;
use crate::commands::registry::StrategyRegistry;
use crate::indices::IndexRegistry;
use crate::plugin::api::events::EventBus;
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
    pub event_bus: Arc<EventBus>,
    pub ui_receiver: broadcast::Receiver<UiMessage>,
    /// Read-only-after-load registry of NSE index constituent lists.
    /// Populated from bundled seed JSON + a background refresh on startup.
    pub index_registry: Arc<IndexRegistry>,
    /// NSE symbol → Dhan `SECURITY_ID` map. Behind an `RwLock` so a future
    /// hot-refresh can swap the map without restarting the app.
    pub symbol_map: Arc<parking_lot::RwLock<SymbolMap>>,
}
