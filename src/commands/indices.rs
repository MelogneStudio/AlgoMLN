use crate::broker::symbol_map::refresh_symbol_map;
use crate::commands::state::AppState;
use crate::indices::{refresh_index, IndexInfo};
use crate::strategy::dsl::ast::IndexAlias;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// Returned by `refresh_indices` IPC command.
#[derive(Debug, Serialize, Deserialize)]
pub struct RefreshResult {
    /// Alias names that were successfully refreshed.
    pub refreshed: Vec<String>,
    /// (alias, error_message) pairs for failures.
    pub failed: Vec<(String, String)>,
    /// Whether the Dhan symbol map was also refreshed.
    pub symbol_map_updated: bool,
    /// New total symbol count in the map after refresh (0 if unchanged).
    pub symbol_map_count: usize,
}

/// Return metadata for all 22 indices (alias, display name, count, last updated).
/// This is a synchronous read — no I/O.
pub fn list_indices(state: &AppState) -> Vec<IndexInfo> {
    state.index_registry.list_info()
}

/// Return the constituent symbol list for a named index alias (e.g. "NIFTY_50").
/// Returns Err if the alias is unrecognised.
pub fn get_index_symbols(state: &AppState, alias: String) -> Result<Vec<String>, String> {
    let parsed = IndexAlias::from_dsl_str(&alias)
        .ok_or_else(|| format!("unknown index alias '{}'", alias))?;
    Ok(state.index_registry.get_symbols(&parsed))
}

/// Refresh all 22 indices from niftyindices.com AND the Dhan scrip master.
/// Writes updated JSON/CSV to app data cache. Non-fatal: errors are reported in the result.
pub async fn refresh_indices(app: &AppHandle, state: &AppState) -> RefreshResult {
    let cache_dir = app.path().app_data_dir().expect("app_data_dir unavailable");

    let indices_cache = cache_dir.join("indices");
    let _ = std::fs::create_dir_all(&indices_cache);

    let registry = Arc::clone(&state.index_registry);

    // Refresh all indices
    let mut refreshed = vec![];
    let mut failed = vec![];
    for alias in IndexAlias::all() {
        let outcome = refresh_index(alias, &indices_cache, &registry).await;
        if outcome.success {
            refreshed.push(outcome.alias);
        } else {
            failed.push((outcome.alias, outcome.error.unwrap_or_default()));
        }
        // Polite delay between requests
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    }

    // Refresh symbol map
    let sym_cache = cache_dir.join("sec_id_cache.csv");
    let (symbol_map_updated, symbol_map_count) = match refresh_symbol_map(&sym_cache).await {
        Ok(new_map) => {
            let count = new_map.len();
            *state.symbol_map.write() = new_map;
            (true, count)
        }
        Err(e) => {
            eprintln!("[refresh_indices] symbol map refresh failed: {}", e);
            (false, 0)
        }
    };

    RefreshResult {
        refreshed,
        failed,
        symbol_map_updated,
        symbol_map_count,
    }
}
