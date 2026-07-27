use std::{
    path::PathBuf,
    sync::Arc,
};

use tauri::AppHandle;
use tokio::sync::{broadcast, Mutex};

use crate::broker::symbol_map::SymbolMap;
use crate::commands::data::DataState;
use crate::commands::registry::StrategyRegistry;
use crate::indices::IndexRegistry;
use crate::live::{
    guard::{LiveGuard, PendingLiveToken},
    session::LiveSession,
    trade_log::TradeLog,
};
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
    /// Append-only live execution audit log, persisted as JSONL.
    pub trade_log: Arc<TradeLog>,
    /// Path to the JSONL audit log for read-only IPC snapshots.
    pub trade_log_path: PathBuf,
    /// Phase-7 live runner: at most one live strategy session is active.
    /// Tokio mutex so the lock can be held across awaits in
    /// `confirm_live_start` without breaking the runtime's `Send` contract.
    pub live_session: Arc<Mutex<Option<Arc<LiveSession>>>>,
    /// Safety gate layer that runs the nine preflight checks before any
    /// live order can be placed. Wired with the same client / symbol map /
    /// broker that the rest of the app uses, plus the path to the
    /// acknowledged-live-trading file.
    pub live_guard: Arc<LiveGuard>,
    /// Single outstanding confirmation token, or `None`. Filled by
    /// `request_live_start`, cleared by `confirm_live_start`. Tokio mutex
    /// so the lock is safely held across awaits in the async command.
    pub pending_live_token: Arc<Mutex<Option<PendingLiveToken>>>,
    /// Path to the JSON file that records the user's one-time
    /// "I understand live trading is risky" consent. Created by
    /// `acknowledge_live_trading`.
    pub ack_path: PathBuf,
    /// Tauri app handle. Needed by `LiveSession::start` for failure
    /// alerts (see `TauriSessionEmitter` in the binary).
    pub app_handle: AppHandle,
}
