use std::collections::HashMap;
use std::path::Path;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::strategy::dsl::ast::IndexAlias;

/// On-disk JSON format for a cached index constituent list.
#[derive(Debug, Serialize, Deserialize)]
pub struct IndexCacheFile {
    pub alias: String,
    /// ISO-8601 date, or "never" if unknown.
    pub last_updated: String,
    pub symbols: Vec<String>,
}

/// Wire type returned to the frontend via IPC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexInfo {
    pub alias: String,        // e.g. "NIFTY_50"
    pub display_name: String, // e.g. "NIFTY 50"
    pub symbol_count: usize,
    pub last_updated: String, // ISO date or "never"
}

struct IndexEntry {
    symbols: Vec<String>,
    last_updated: String,
}

/// Read-only-after-load registry of NSE index constituent lists.
///
/// `IndexRegistry::update` is the only mutator and is intended to be called
/// only by `refresh_index` at startup. The strategy engine reads constituents
/// once at deploy time and does not re-read mid-run — see invariant #10
/// in `CLAUDE.md`.
pub struct IndexRegistry {
    data: RwLock<HashMap<IndexAlias, IndexEntry>>,
}

impl IndexRegistry {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(HashMap::new()),
        }
    }

    /// Load index constituent files from `cache_dir` (user's app data).
    /// Falls back to `resource_dir` (bundled seed) for any file not found in
    /// cache. Logs to stderr for any index that fails to load from both
    /// locations. Inserts an empty entry for missing keys so callers can
    /// distinguish "loaded zero" from "not loaded at all".
    pub fn load_from_dirs(&self, cache_dir: &Path, resource_dir: &Path) {
        let mut data = self.data.write();
        for alias in IndexAlias::all() {
            let filename = format!("{}.json", alias.cache_stem());
            let cache_path = cache_dir.join(&filename);
            let resource_path = resource_dir.join(&filename);

            let loaded = Self::try_load_file(&cache_path)
                .or_else(|| Self::try_load_file(&resource_path));

            match loaded {
                Some(file) => {
                    data.insert(
                        alias.clone(),
                        IndexEntry {
                            symbols: file.symbols,
                            last_updated: file.last_updated,
                        },
                    );
                }
                None => {
                    eprintln!(
                        "[IndexRegistry] no data for {} (tried {} and {})",
                        alias.display_name(),
                        cache_path.display(),
                        resource_path.display()
                    );
                    // Insert empty entry so the key always exists.
                    data.insert(
                        alias.clone(),
                        IndexEntry {
                            symbols: vec![],
                            last_updated: "never".to_string(),
                        },
                    );
                }
            }
        }
    }

    fn try_load_file(path: &Path) -> Option<IndexCacheFile> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// Returns the constituent symbol list for an alias. Empty vec if not loaded.
    pub fn get_symbols(&self, alias: &IndexAlias) -> Vec<String> {
        self.data
            .read()
            .get(alias)
            .map(|e| e.symbols.clone())
            .unwrap_or_default()
    }

    /// Update an alias entry after a successful refresh.
    pub fn update(&self, alias: IndexAlias, symbols: Vec<String>, last_updated: String) {
        self.data.write().insert(
            alias,
            IndexEntry {
                symbols,
                last_updated,
            },
        );
    }

    /// List info for all 22 indices (for IPC / UI display).
    pub fn list_info(&self) -> Vec<IndexInfo> {
        let data = self.data.read();
        IndexAlias::all()
            .iter()
            .map(|alias| {
                let entry = data.get(alias);
                IndexInfo {
                    alias: alias.dsl_keyword().to_string(),
                    display_name: alias.display_name().to_string(),
                    symbol_count: entry.map(|e| e.symbols.len()).unwrap_or(0),
                    last_updated: entry
                        .map(|e| e.last_updated.clone())
                        .unwrap_or_else(|| "never".to_string()),
                }
            })
            .collect()
    }
}

impl Default for IndexRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_aliases_have_csv_filename() {
        for alias in IndexAlias::all() {
            assert!(!alias.csv_filename().is_empty());
            assert!(!alias.cache_stem().is_empty());
            assert!(!alias.dsl_keyword().is_empty());
        }
    }

    #[test]
    fn from_dsl_str_roundtrip() {
        for alias in IndexAlias::all() {
            let kw = alias.dsl_keyword();
            let parsed = IndexAlias::from_dsl_str(kw);
            assert_eq!(parsed.as_ref(), Some(alias), "roundtrip failed for {}", kw);
        }
    }

    #[test]
    fn list_info_returns_all_22_indices() {
        let registry = IndexRegistry::new();
        let info = registry.list_info();
        assert_eq!(info.len(), 22);
    }

    #[test]
    fn update_then_get_symbols_roundtrip() {
        let registry = IndexRegistry::new();
        registry.update(
            IndexAlias::NiftyBank,
            vec!["HDFCBANK".to_string(), "ICICIBANK".to_string()],
            "2026-07-17".to_string(),
        );
        let symbols = registry.get_symbols(&IndexAlias::NiftyBank);
        assert_eq!(symbols, vec!["HDFCBANK", "ICICIBANK"]);
    }
}
