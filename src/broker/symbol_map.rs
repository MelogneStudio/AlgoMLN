use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

const DHAN_SCRIP_MASTER_URL: &str = "https://images.dhan.co/api-data/api-scrip-master-detailed.csv";

/// Exchange segment for a tradable symbol. Phase 7 supports only `NseEq` for
/// order placement; the other variants exist so the segment guard can name a
/// rejected symbol and so Phase 8 can relax the restriction without a schema
/// change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Segment {
    NseEq,
    NseFno,
    NseCurrency,
    Bse,
    Mcx,
    Index,
}

impl Segment {
    /// The Dhan `exchangeSegment` string for this segment.
    pub fn as_dhan_string(self) -> &'static str {
        match self {
            Segment::NseEq => "NSE_EQ",
            Segment::NseFno => "NSE_FNO",
            Segment::NseCurrency => "NSE_CURRENCY",
            Segment::Bse => "BSE_EQ",
            Segment::Mcx => "MCX_COMM",
            Segment::Index => "IDX_I",
        }
    }
}

/// A resolved symbol: its Dhan security id and the segment it trades in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolEntry {
    pub security_id: u32,
    pub segment: Segment,
}

/// Maps NSE equity trading symbols to Dhan security IDs.
/// Loaded once at startup; shared via Arc.
#[derive(Debug)]
pub struct SymbolMap {
    /// key: uppercase NSE symbol, value: Dhan SECURITY_ID
    map: HashMap<String, u32>,
    /// key: uppercase NSE symbol, value: exchange segment. Kept parallel to
    /// `map` so `inner()` / `get()` retain their `u32` shape for the fuzzy
    /// search path; `lookup()` joins the two.
    segments: HashMap<String, Segment>,
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
        let mut segments: HashMap<String, Segment> = HashMap::new();
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
                    map.insert(key.clone(), sec_id);
                    // The CSV filter above admits only NSE equities, so every
                    // parsed entry is NseEq.
                    segments.insert(key, Segment::NseEq);
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
        Ok(Self { map, segments })
    }

    /// Empty map — used when the seed file is unavailable so the app still boots.
    pub fn empty() -> Self {
        Self {
            map: HashMap::new(),
            segments: HashMap::new(),
        }
    }

    /// Look up a security ID for a symbol. Case-insensitive.
    pub fn get(&self, symbol: &str) -> Option<u32> {
        self.map.get(&symbol.trim().to_uppercase()).copied()
    }

    /// Look up the full entry (security id + segment) for a symbol.
    /// Case-insensitive. If the security id is present but the segment is
    /// somehow missing, defaults to `NseEq` with a warning rather than
    /// dropping the symbol.
    pub fn lookup(&self, symbol: &str) -> Option<SymbolEntry> {
        let key = symbol.trim().to_uppercase();
        let security_id = *self.map.get(&key)?;
        let segment = match self.segments.get(&key) {
            Some(segment) => *segment,
            None => {
                eprintln!("[SymbolMap] segment missing for {key}; defaulting to NseEq");
                Segment::NseEq
            }
        };
        Some(SymbolEntry {
            security_id,
            segment,
        })
    }

    /// Insert or overwrite a single entry. Used by tests and any future
    /// non-CSV loader; the CSV path populates entries as NSE equities.
    pub fn insert_entry(&mut self, symbol: &str, security_id: u32, segment: Segment) {
        let key = symbol.trim().to_uppercase();
        self.map.insert(key.clone(), security_id);
        self.segments.insert(key, segment);
    }

    /// Borrow the raw symbol → security_id map. Used by the
    /// `search_symbols` IPC command to walk the entire universe.
    pub fn inner(&self) -> &HashMap<String, u32> {
        &self.map
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

    #[test]
    fn parsed_entries_are_nse_equity() {
        let csv = "EXCH_ID,SEGMENT,SYMBOL_NAME,SECURITY_ID\n\
                   NSE,E,RELIANCE,2885\n";
        let map = SymbolMap::parse_csv(csv).unwrap();
        let entry = map.lookup("reliance").expect("case-insensitive lookup");
        assert_eq!(entry.security_id, 2885);
        assert_eq!(entry.segment, Segment::NseEq);
    }

    #[test]
    fn insert_entry_records_segment() {
        let mut map = SymbolMap::empty();
        map.insert_entry("BANKNIFTY", 1234, Segment::NseFno);
        let entry = map.lookup("BANKNIFTY").unwrap();
        assert_eq!(entry.security_id, 1234);
        assert_eq!(entry.segment, Segment::NseFno);
        // `get` still returns the security id for the fuzzy-search path.
        assert_eq!(map.get("BANKNIFTY"), Some(1234));
    }

    #[test]
    fn segment_as_dhan_string() {
        assert_eq!(Segment::NseEq.as_dhan_string(), "NSE_EQ");
        assert_eq!(Segment::NseFno.as_dhan_string(), "NSE_FNO");
    }
}
