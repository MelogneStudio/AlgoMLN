use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;

const DHAN_SCRIP_MASTER_URL: &str =
    "https://images.dhan.co/api-data/api-scrip-master-detailed.csv";

/// Maps NSE equity trading symbols to Dhan security IDs.
/// Loaded once at startup; shared via Arc.
pub struct SymbolMap {
    /// key: uppercase NSE symbol, value: Dhan SECURITY_ID
    map: HashMap<String, u32>,
}

#[derive(Debug, Deserialize)]
struct ScripRow {
    #[serde(rename = "EXCH_ID")]
    exch_id: String,
    #[serde(rename = "SEGMENT")]
    segment: String,
    /// Primary: present in detailed CSV for equities.
    #[serde(rename = "SYMBOL_NAME", default)]
    symbol_name: Option<String>,
    /// Fallback: present for equities in some CSV versions (also used for
    /// derivative underlyings).
    #[serde(rename = "UNDERLYING_SYMBOL", default)]
    underlying_symbol: Option<String>,
    #[serde(rename = "SECURITY_ID")]
    security_id: Option<u32>,
}

impl SymbolMap {
    /// Load from a CSV file (either seed or cache). Returns an error if the
    /// file is missing or unparseable. Does NOT fall back — callers handle
    /// fallback.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        // Strip BOM.
        let text = text.trim_start_matches('\u{feff}');
        Self::parse_csv(text)
    }

    fn parse_csv(text: &str) -> Result<Self, String> {
        let mut rdr = csv::Reader::from_reader(text.as_bytes());
        let mut map: HashMap<String, u32> = HashMap::new();
        let mut duplicates = 0usize;

        for result in rdr.deserialize::<ScripRow>() {
            let row = match result {
                Ok(r) => r,
                Err(_) => continue, // skip unparseable rows silently
            };

            // Filter: NSE equities only.
            if row.exch_id.trim() != "NSE" || row.segment.trim() != "E" {
                continue;
            }

            let sec_id = match row.security_id {
                Some(id) => id,
                None => continue,
            };

            // Prefer SYMBOL_NAME, fall back to UNDERLYING_SYMBOL.
            let symbol = row
                .symbol_name
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| {
                    row.underlying_symbol
                        .as_deref()
                        .filter(|s| !s.trim().is_empty())
                });

            if let Some(sym) = symbol {
                let key = sym.trim().to_uppercase();
                if map.contains_key(&key) {
                    duplicates += 1;
                    // First occurrence wins (matches the user's Python script).
                } else {
                    map.insert(key, sec_id);
                }
            }
        }

        if duplicates > 0 {
            eprintln!(
                "[SymbolMap] {} duplicate symbols ignored (first-wins)",
                duplicates
            );
        }
        eprintln!("[SymbolMap] loaded {} NSE equity symbols", map.len());
        Ok(Self { map })
    }

    /// Empty map — used when the seed file is unavailable so the app still boots.
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
        }
    }

    /// Look up a security ID for a symbol. Case-insensitive.
    pub fn get(&self, symbol: &str) -> Option<u32> {
        self.map.get(&symbol.trim().to_uppercase()).copied()
    }

    /// Batch lookup. Returns (found: Vec<(symbol, security_id)>, missing: Vec<symbol>).
    pub fn resolve_many(&self, symbols: &[String]) -> (Vec<(String, u32)>, Vec<String>) {
        let mut found = Vec::with_capacity(symbols.len());
        let mut missing = Vec::new();
        for sym in symbols {
            match self.get(sym) {
                Some(id) => found.push((sym.clone(), id)),
                None => missing.push(sym.clone()),
            }
        }
        (found, missing)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Download the Dhan scrip master CSV, write to `cache_path`, return a loaded
/// SymbolMap. On any failure, returns Err — callers fall back to the seed
/// file.
pub async fn refresh_symbol_map(cache_path: &Path) -> Result<SymbolMap, String> {
    eprintln!("[SymbolMap] downloading scrip master from Dhan…");
    let response = reqwest::Client::builder()
        .user_agent("AlgoMLN/1.0")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?
        .get(DHAN_SCRIP_MASTER_URL)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;

    if !response.status().is_success() {
        return Err(format!(
            "Dhan scrip master returned HTTP {}",
            response.status()
        ));
    }

    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    // Atomic write via temp file.
    let tmp = cache_path.with_extension("tmp");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, cache_path).map_err(|e| e.to_string())?;

    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}');
    let map = SymbolMap::parse_csv(text)?;
    eprintln!("[SymbolMap] refreshed: {} symbols", map.len());
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_csv() {
        let csv = "EXCH_ID,SEGMENT,SYMBOL_NAME,SECURITY_ID\n\
                   NSE,E,RELIANCE,2885\n\
                   NSE,D,NIFTY,0\n\
                   BSE,E,RELIANCE,500325\n";
        let map = SymbolMap::parse_csv(csv).unwrap();
        assert_eq!(map.get("RELIANCE"), Some(2885));
        assert_eq!(map.get("reliance"), Some(2885)); // case-insensitive
        assert_eq!(map.get("NIFTY"), None); // derivative filtered out
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn empty_map_is_empty() {
        let map = SymbolMap::empty();
        assert!(map.is_empty());
        assert_eq!(map.get("RELIANCE"), None);
    }

    #[test]
    fn resolve_many_splits_found_and_missing() {
        let map = SymbolMap::empty();
        let symbols = vec!["A".to_string(), "B".to_string()];
        let (found, missing) = map.resolve_many(&symbols);
        assert!(found.is_empty());
        assert_eq!(missing, symbols);
    }
}
