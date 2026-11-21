use std::io::Read;
use std::path::Path;
use std::time::Duration;

use crate::indices::registry::{IndexAlias, IndexCacheFile, IndexRegistry};

pub const DEFAULT_BASE_URL: &str = "https://www.niftyindices.com/IndexConstituent/";
pub const BROWSER_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[derive(Debug, thiserror::Error)]
pub enum RefreshError {
    #[error("HTTP {status} fetching {url}")]
    HttpStatus { status: u16, url: String },

    #[error("Network error fetching {url}: {source}")]
    Network {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("CSV parse error for {alias}: {message}")]
    Parse { alias: String, message: String },

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefreshOutcome {
    pub alias: String,
    pub display_name: String,
    pub success: bool,
    pub symbol_count: usize,
    pub as_of: String,
    pub error: Option<String>,
}

/// Atomically writes cache JSON file to `cache_dir`:
/// writes to a temporary file in the same directory, then renames it over the target file.
/// If any write error occurs, the temporary file is removed and target remains untouched.
pub fn write_cache_atomic(
    cache_dir: &Path,
    alias: IndexAlias,
    cache_file: &IndexCacheFile,
) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(cache_dir)?;
    let target = cache_dir.join(format!("{}.json", alias.alias()));
    let temp = cache_dir.join(format!(
        "{}.json.tmp.{}_{}",
        alias.alias(),
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    let json_bytes = serde_json::to_vec_pretty(cache_file)?;
    std::fs::write(&temp, json_bytes)?;

    match std::fs::rename(&temp, &target) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

/// Downloads CSV from niftyindices.com (or mock `base_url`), parses constituents,
/// and atomically updates `<alias>.json` cache in `cache_dir` and the in-memory registry.
///
/// Any HTTP error, parse error, or network error leaves any existing cache file on disk untouched.
pub fn refresh_index(
    alias: IndexAlias,
    cache_dir: &Path,
    registry: Option<&IndexRegistry>,
    base_url: Option<&str>,
    client: Option<&reqwest::blocking::Client>,
) -> Result<IndexCacheFile, RefreshError> {
    let base = base_url.unwrap_or(DEFAULT_BASE_URL);
    let url = format!("{}{}", base, alias.csv_filename());

    let default_client;
    let client = match client {
        Some(c) => c,
        None => {
            default_client = reqwest::blocking::Client::builder()
                .user_agent(BROWSER_USER_AGENT)
                .timeout(Duration::from_secs(20))
                .build()
                .map_err(|e| RefreshError::Network {
                    url: url.clone(),
                    source: e,
                })?;
            &default_client
        }
    };

    let response = client
        .get(&url)
        .header(reqwest::header::USER_AGENT, BROWSER_USER_AGENT)
        .send()
        .map_err(|e| RefreshError::Network {
            url: url.clone(),
            source: e,
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(RefreshError::HttpStatus {
            status: status.as_u16(),
            url,
        });
    }

    let mut body = String::new();
    let mut reader = response;
    reader
        .read_to_string(&mut body)
        .map_err(|e| RefreshError::Parse {
            alias: alias.dsl_keyword().to_string(),
            message: format!("failed to read response body: {e}"),
        })?;

    // Validate not an HTML error page (NSE server sometimes returns 200 with HTML)
    let body_trimmed = body.trim_start_matches('\u{feff}').trim();
    if body_trimmed.starts_with("<!DOCTYPE")
        || body_trimmed.starts_with("<html")
        || body_trimmed.starts_with("<?xml")
    {
        return Err(RefreshError::Parse {
            alias: alias.dsl_keyword().to_string(),
            message: "received HTML response instead of CSV".to_string(),
        });
    }

    let symbols = parse_symbols_from_csv(alias, body_trimmed)?;
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();

    let cache_file = IndexCacheFile {
        alias: alias.dsl_keyword().to_string(),
        as_of: today,
        source_url: Some(url),
        symbols,
    };

    // Atomic write to disk
    write_cache_atomic(cache_dir, alias, &cache_file)?;

    // Update in-memory registry if provided
    if let Some(reg) = registry {
        reg.update_entry(alias, cache_file.clone());
    }

    Ok(cache_file)
}

/// Parses the "Symbol" column out of niftyindices CSV data.
pub fn parse_symbols_from_csv(
    alias: IndexAlias,
    csv_text: &str,
) -> Result<Vec<String>, RefreshError> {
    let text = csv_text.trim_start_matches('\u{feff}');
    let mut rdr = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(text.as_bytes());

    let headers = rdr.headers().map_err(|e| RefreshError::Parse {
        alias: alias.dsl_keyword().to_string(),
        message: format!("malformed CSV headers: {e}"),
    })?;

    let symbol_col = headers
        .iter()
        .position(|h| h.trim().eq_ignore_ascii_case("Symbol"))
        .ok_or_else(|| RefreshError::Parse {
            alias: alias.dsl_keyword().to_string(),
            message: "CSV has no 'Symbol' header column".to_string(),
        })?;

    let mut symbols = Vec::new();
    for record_res in rdr.records() {
        let rec = record_res.map_err(|e| RefreshError::Parse {
            alias: alias.dsl_keyword().to_string(),
            message: format!("malformed CSV record: {e}"),
        })?;

        if let Some(sym) = rec.get(symbol_col) {
            let s = sym.trim();
            if !s.is_empty() {
                symbols.push(s.to_ascii_uppercase());
            }
        }
    }

    if symbols.is_empty() {
        return Err(RefreshError::Parse {
            alias: alias.dsl_keyword().to_string(),
            message: "CSV parsed but contained no constituent symbols".to_string(),
        });
    }

    Ok(symbols)
}

/// Refreshes all 22 indices. A failure on one index does NOT abort the remaining ones.
pub fn refresh_all_indices(
    registry: &IndexRegistry,
    cache_dir: &Path,
    base_url: Option<&str>,
    client: Option<&reqwest::blocking::Client>,
) -> Vec<RefreshOutcome> {
    let mut outcomes = Vec::with_capacity(IndexAlias::all().len());

    for alias in IndexAlias::all() {
        match refresh_index(*alias, cache_dir, Some(registry), base_url, client) {
            Ok(file) => {
                outcomes.push(RefreshOutcome {
                    alias: alias.alias().to_string(),
                    display_name: alias.display_name().to_string(),
                    success: true,
                    symbol_count: file.symbols.len(),
                    as_of: file.as_of,
                    error: None,
                });
            }
            Err(e) => {
                let current_entry = registry.get_entry(*alias);
                outcomes.push(RefreshOutcome {
                    alias: alias.alias().to_string(),
                    display_name: alias.display_name().to_string(),
                    success: false,
                    symbol_count: current_entry.as_ref().map(|e| e.symbols.len()).unwrap_or(0),
                    as_of: current_entry
                        .as_ref()
                        .map(|e| e.as_of.clone())
                        .unwrap_or_else(|| "never".to_string()),
                    error: Some(e.to_string()),
                });
            }
        }
    }

    outcomes
}
