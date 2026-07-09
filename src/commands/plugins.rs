use crate::commands::state::AppState;
use crate::plugin::{PluginId, PluginListEntry};

/// List every loaded plugin. Returns a snapshot suitable for direct
/// serialization to the React app (the `PluginListEntry` wire type
/// already derives `Serialize`).
pub async fn list_plugins(state: &AppState) -> Result<Vec<PluginListEntry>, String> {
    Ok(state.plugin_registry.list())
}

/// Move a loaded plugin into the `Enabled` state.
pub async fn enable_plugin(state: &AppState, id: String) -> Result<(), String> {
    state
        .plugin_registry
        .enable(&PluginId(id))
        .await
        .map_err(|e| e.to_string())
}

/// Move an enabled plugin back to `Disabled`.
pub async fn disable_plugin(state: &AppState, id: String) -> Result<(), String> {
    state
        .plugin_registry
        .disable(&PluginId(id))
        .await
        .map_err(|e| e.to_string())
}

/// Re-scan the plugins directory, loading any new entries and replacing
/// the registry's view of existing ones. Returns a list of per-plugin
/// error messages (the empty list means every plugin loaded cleanly).
pub async fn reload_plugins(state: &AppState) -> Result<Vec<String>, String> {
    let results = state.plugin_registry.scan_and_load().await;
    Ok(results
        .into_iter()
        .filter_map(|(id, r)| r.err().map(|e| format!("{id}: {e}")))
        .collect())
}
