use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::strategy::dsl::ast::IndexAlias;

use super::registry::{IndexCacheFile, IndexRegistry};

const BASE_URL: &str = "https://www.niftyindices.com/IndexConstituent/";

/// Default staleness threshold: a cache older than 24h triggers a refresh.
pub const DEFAULT_STALENESS: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug)]
pub struct RefreshOutcome {
    pub alias: String,
    pub success: bool,
    pub symbol_count: usize,
    pub error: Option<String>,
}

/// Fetch a single index from niftyindices.com, parse it, write the JSON
/// cache, and update the registry. Returns the outcome.
pub async fn refresh_index(
    alias: &IndexAlias,
    cache_dir: &Path,
    registry: &IndexRegistry,
) -> RefreshOutcome {
    let url = format!("{}{}", BASE_URL, alias.csv_filename());
    let alias_str = alias.dsl_keyword().to_string();

    match fetch_and_parse(&url).await {
        Ok(symbols) => {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let cache_file = IndexCacheFile {
                alias: alias_str.clone(),
                last_updated: today.clone(),
                symbols: symbols.clone(),
            };
            let out_path = cache_dir.join(format!("{}.json", alias.cache_stem()));
            if let Err(e) = std::fs::create_dir_all(cache_dir) {
                eprintln!("[refresh_index] could not create cache dir: {}", e);
            }
            match serde_json::to_string_pretty(&cache_file) {
                Ok(json) => {
                    if let Err(e) = std::fs::write(&out_path, json) {
                        eprintln!(
                            "[refresh_index] could not write cache for {}: {}",
                            alias_str, e
                        );
                    }
                }
                Err(e) => eprintln!("[refresh_index] could not serialise {}: {}", alias_str, e),
            }
            registry.update(alias.clone(), symbols.clone(), today);
            let count = symbols.len();
            eprintln!("[refresh_index] {} → {} symbols", alias_str, count);
            RefreshOutcome {
                alias: alias_str,
                success: true,
                symbol_count: count,
                error: None,
            }
        }
        Err(e) => {
            eprintln!("[refresh_index] failed for {}: {}", alias_str, e);
            RefreshOutcome {
                alias: alias_str,
                success: false,
                symbol_count: 0,
                error: Some(e),
            }
        }
    }
}

async fn fetch_and_parse(url: &str) -> Result<Vec<String>, String> {
    let response = reqwest::Client::builder()
        .user_agent("AlgoMLN/1.0")
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("HTTP error: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let text = response.text().await.map_err(|e| e.to_string())?;
    // Strip UTF-8 BOM if present.
    let text = text.trim_start_matches('\u{feff}');

    let mut rdr = csv::Reader::from_reader(text.as_bytes());
    let headers = rdr.headers().map_err(|e| e.to_string())?.clone();

    // Find the "Symbol" column index (niftyindices CSVs use this exact header).
    let symbol_col = headers
        .iter()
        .position(|h| h.trim() == "Symbol")
        .ok_or("CSV has no 'Symbol' column")?;

    let symbols: Vec<String> = rdr
        .records()
        .filter_map(|r| r.ok())
        .filter_map(|r| r.get(symbol_col).map(|s| s.trim().to_uppercase()))
        .filter(|s| !s.is_empty())
        .collect();

    if symbols.is_empty() {
        return Err("CSV parsed but contained no symbols".to_string());
    }
    Ok(symbols)
}

/// Check if `path` is older than `threshold` (or missing). Returns true if
/// refresh is needed.
pub fn is_stale(path: &Path, threshold: Duration) -> bool {
    path.metadata()
        .and_then(|m| m.modified())
        .map(|modified| {
            SystemTime::now()
                .duration_since(modified)
                .unwrap_or(Duration::MAX)
                > threshold
        })
        .unwrap_or(true)
}

/// Refresh all 22 indices if the oldest cache file is stale (> threshold).
/// Called in background on startup. Non-fatal: failures logged to stderr.
pub async fn refresh_all_if_stale(
    registry: Arc<IndexRegistry>,
    cache_dir: std::path::PathBuf,
    threshold: Duration,
) -> Vec<RefreshOutcome> {
    // Check staleness by looking at the nifty_50.json cache file as a proxy.
    let proxy = cache_dir.join("nifty_50.json");
    if !is_stale(&proxy, threshold) {
        return vec![]; // fresh enough, skip
    }

    eprintln!("[refresh_all_if_stale] indices are stale, refreshing all…");
    let mut outcomes = vec![];
    for alias in IndexAlias::all() {
        let outcome = refresh_index(alias, &cache_dir, &registry).await;
        outcomes.push(outcome);
        // Small delay between requests to be polite to niftyindices.com.
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    outcomes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_stale_returns_true_for_missing_path() {
        let path = Path::new("/tmp/algomln-test-no-such-file-12345");
        assert!(is_stale(path, Duration::from_secs(60)));
    }
}
