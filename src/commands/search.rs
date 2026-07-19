//! Tauri command body for the `search_symbols` IPC.
//!
//! The fuzzy-search logic itself lives in `crate::search` and is exercised
//! directly by unit tests. This file is just the async body that pulls the
//! data the scorer needs out of `AppState` and forwards the result.
//!
//! **Lock discipline.** The body takes read guards on `symbol_map` and
//! `index_registry`, clones the data it needs into owned values, and
//! drops both guards BEFORE calling `fuzzy_search`. parking_lot's
//! `RwLockReadGuard` is `!Send` and must not be held across `.await` —
//! there is no `.await` in this function today, but the pattern keeps it
//! safe for future refactors.

use crate::commands::state::AppState;
use crate::search::{fuzzy_search, SymbolMatch};

/// Fuzzy-search the symbol universe (equities + indices).
///
/// `max_results` is hard-coded to 5 here; if a future caller wants a
/// different cap, the parameter should be added to the Tauri wrapper
/// (not a magic constant) and forwarded.
pub async fn search_symbols_impl(
    state: &AppState,
    query: String,
) -> Result<Vec<SymbolMatch>, String> {
    // Clone everything we need under short-lived guards, then drop the
    // guards. This keeps the function `Send` and avoids holding any
    // parking_lot guard across an `.await` (none today, but the rule
    // still applies).
    //
    // `state.symbol_map` is `Arc<parking_lot::RwLock<SymbolMap>>` so we
    // call `.read()` on the inner `RwLock` to get a guard, then borrow
    // the underlying map. `state.index_registry` is `Arc<IndexRegistry>`
    // — `IndexRegistry::collect_entries` already takes the read guard
    // internally and returns an owned `Vec<IndexEntry>`, which is the
    // safest shape to forward into a pure search function.
    let (equity_map, index_entries) = {
        let sym_map = state.symbol_map.read();
        let entries = state.index_registry.collect_entries();
        (sym_map.inner().clone(), entries)
    };

    Ok(fuzzy_search(&query, &equity_map, &index_entries, 5))
}
