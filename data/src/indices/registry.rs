use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

/// The 22 supported NSE index aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum IndexAlias {
    Nifty50,
    NiftyNext50,
    Nifty100,
    Nifty200,
    Nifty500,
    NiftyMidcap50,
    NiftyMidcap100,
    NiftyMidcap150,
    NiftySmallcap50,
    NiftySmallcap100,
    NiftySmallcap250,
    NiftyBank,
    NiftyIt,
    NiftyPharma,
    NiftyAuto,
    NiftyFmcg,
    NiftyMetal,
    NiftyRealty,
    NiftyEnergy,
    NiftyInfra,
    NiftyPsuBank,
    NiftyFinancialServices,
}

impl IndexAlias {
    /// All 22 index aliases in stable canonical order.
    pub const ALL: &'static [IndexAlias] = &[
        IndexAlias::Nifty50,
        IndexAlias::NiftyNext50,
        IndexAlias::Nifty100,
        IndexAlias::Nifty200,
        IndexAlias::Nifty500,
        IndexAlias::NiftyMidcap50,
        IndexAlias::NiftyMidcap100,
        IndexAlias::NiftyMidcap150,
        IndexAlias::NiftySmallcap50,
        IndexAlias::NiftySmallcap100,
        IndexAlias::NiftySmallcap250,
        IndexAlias::NiftyBank,
        IndexAlias::NiftyIt,
        IndexAlias::NiftyPharma,
        IndexAlias::NiftyAuto,
        IndexAlias::NiftyFmcg,
        IndexAlias::NiftyMetal,
        IndexAlias::NiftyRealty,
        IndexAlias::NiftyEnergy,
        IndexAlias::NiftyInfra,
        IndexAlias::NiftyPsuBank,
        IndexAlias::NiftyFinancialServices,
    ];

    pub fn all() -> &'static [IndexAlias] {
        Self::ALL
    }

    /// Lowercase file stem used for on-disk cache (e.g. "nifty_50").
    pub fn alias(&self) -> &'static str {
        match self {
            Self::Nifty50 => "nifty_50",
            Self::NiftyNext50 => "nifty_next_50",
            Self::Nifty100 => "nifty_100",
            Self::Nifty200 => "nifty_200",
            Self::Nifty500 => "nifty_500",
            Self::NiftyMidcap50 => "nifty_midcap_50",
            Self::NiftyMidcap100 => "nifty_midcap_100",
            Self::NiftyMidcap150 => "nifty_midcap_150",
            Self::NiftySmallcap50 => "nifty_smallcap_50",
            Self::NiftySmallcap100 => "nifty_smallcap_100",
            Self::NiftySmallcap250 => "nifty_smallcap_250",
            Self::NiftyBank => "nifty_bank",
            Self::NiftyIt => "nifty_it",
            Self::NiftyPharma => "nifty_pharma",
            Self::NiftyAuto => "nifty_auto",
            Self::NiftyFmcg => "nifty_fmcg",
            Self::NiftyMetal => "nifty_metal",
            Self::NiftyRealty => "nifty_realty",
            Self::NiftyEnergy => "nifty_energy",
            Self::NiftyInfra => "nifty_infra",
            Self::NiftyPsuBank => "nifty_psu_bank",
            Self::NiftyFinancialServices => "nifty_financial_services",
        }
    }

    /// Human-readable display name (e.g. "NIFTY 50").
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Nifty50 => "NIFTY 50",
            Self::NiftyNext50 => "NIFTY NEXT 50",
            Self::Nifty100 => "NIFTY 100",
            Self::Nifty200 => "NIFTY 200",
            Self::Nifty500 => "NIFTY 500",
            Self::NiftyMidcap50 => "NIFTY MIDCAP 50",
            Self::NiftyMidcap100 => "NIFTY MIDCAP 100",
            Self::NiftyMidcap150 => "NIFTY MIDCAP 150",
            Self::NiftySmallcap50 => "NIFTY SMALLCAP 50",
            Self::NiftySmallcap100 => "NIFTY SMALLCAP 100",
            Self::NiftySmallcap250 => "NIFTY SMALLCAP 250",
            Self::NiftyBank => "NIFTY BANK",
            Self::NiftyIt => "NIFTY IT",
            Self::NiftyPharma => "NIFTY PHARMA",
            Self::NiftyAuto => "NIFTY AUTO",
            Self::NiftyFmcg => "NIFTY FMCG",
            Self::NiftyMetal => "NIFTY METAL",
            Self::NiftyRealty => "NIFTY REALTY",
            Self::NiftyEnergy => "NIFTY ENERGY",
            Self::NiftyInfra => "NIFTY INFRA",
            Self::NiftyPsuBank => "NIFTY PSU BANK",
            Self::NiftyFinancialServices => "NIFTY FINANCIAL SERVICES",
        }
    }

    /// Standard DSL keyword representation (e.g. "NIFTY_50").
    pub fn dsl_keyword(&self) -> &'static str {
        match self {
            Self::Nifty50 => "NIFTY_50",
            Self::NiftyNext50 => "NIFTY_NEXT_50",
            Self::Nifty100 => "NIFTY_100",
            Self::Nifty200 => "NIFTY_200",
            Self::Nifty500 => "NIFTY_500",
            Self::NiftyMidcap50 => "NIFTY_MIDCAP_50",
            Self::NiftyMidcap100 => "NIFTY_MIDCAP_100",
            Self::NiftyMidcap150 => "NIFTY_MIDCAP_150",
            Self::NiftySmallcap50 => "NIFTY_SMALLCAP_50",
            Self::NiftySmallcap100 => "NIFTY_SMALLCAP_100",
            Self::NiftySmallcap250 => "NIFTY_SMALLCAP_250",
            Self::NiftyBank => "NIFTY_BANK",
            Self::NiftyIt => "NIFTY_IT",
            Self::NiftyPharma => "NIFTY_PHARMA",
            Self::NiftyAuto => "NIFTY_AUTO",
            Self::NiftyFmcg => "NIFTY_FMCG",
            Self::NiftyMetal => "NIFTY_METAL",
            Self::NiftyRealty => "NIFTY_REALTY",
            Self::NiftyEnergy => "NIFTY_ENERGY",
            Self::NiftyInfra => "NIFTY_INFRA",
            Self::NiftyPsuBank => "NIFTY_PSU_BANK",
            Self::NiftyFinancialServices => "NIFTY_FINANCIAL_SERVICES",
        }
    }

    /// Official niftyindices.com CSV filename.
    pub fn csv_filename(&self) -> &'static str {
        match self {
            Self::Nifty50 => "ind_nifty50list.csv",
            Self::NiftyNext50 => "ind_niftynext50list.csv",
            Self::Nifty100 => "ind_nifty100list.csv",
            Self::Nifty200 => "ind_nifty200list.csv",
            Self::Nifty500 => "ind_nifty500list.csv",
            Self::NiftyMidcap50 => "ind_niftymidcap50list.csv",
            Self::NiftyMidcap100 => "ind_niftymidcap100list.csv",
            Self::NiftyMidcap150 => "ind_niftymidcap150list.csv",
            Self::NiftySmallcap50 => "ind_niftysmallcap50list.csv",
            Self::NiftySmallcap100 => "ind_niftysmallcap100list.csv",
            Self::NiftySmallcap250 => "ind_niftysmallcap250list.csv",
            Self::NiftyBank => "ind_niftybanklist.csv",
            Self::NiftyIt => "ind_niftyitlist.csv",
            Self::NiftyPharma => "ind_niftypharmaList.csv",
            Self::NiftyAuto => "ind_niftyautolist.csv",
            Self::NiftyFmcg => "ind_niftyfmcglist.csv",
            Self::NiftyMetal => "ind_niftymetallist.csv",
            Self::NiftyRealty => "ind_niftyrealtylist.csv",
            Self::NiftyEnergy => "ind_niftyenergylist.csv",
            Self::NiftyInfra => "ind_niftyinfraList.csv",
            Self::NiftyPsuBank => "ind_niftypsubanklist.csv",
            Self::NiftyFinancialServices => "ind_niftyfinancelist.csv",
        }
    }

    /// Case-insensitive keyword lookup matching with or without underscores/hyphens/spaces.
    /// E.g. "NIFTYBANK", "nifty_bank", "NIFTY BANK" all resolve to `IndexAlias::NiftyBank`.
    pub fn from_keyword(s: &str) -> Option<Self> {
        let norm: String = s
            .trim()
            .to_ascii_uppercase()
            .chars()
            .filter(|c| !matches!(c, '_' | '-' | ' '))
            .collect();

        match norm.as_str() {
            "NIFTY50" => Some(Self::Nifty50),
            "NIFTYNEXT50" => Some(Self::NiftyNext50),
            "NIFTY100" => Some(Self::Nifty100),
            "NIFTY200" => Some(Self::Nifty200),
            "NIFTY500" => Some(Self::Nifty500),
            "NIFTYMIDCAP50" => Some(Self::NiftyMidcap50),
            "NIFTYMIDCAP100" => Some(Self::NiftyMidcap100),
            "NIFTYMIDCAP150" => Some(Self::NiftyMidcap150),
            "NIFTYSMALLCAP50" => Some(Self::NiftySmallcap50),
            "NIFTYSMALLCAP100" => Some(Self::NiftySmallcap100),
            "NIFTYSMALLCAP250" => Some(Self::NiftySmallcap250),
            "NIFTYBANK" => Some(Self::NiftyBank),
            "NIFTYIT" => Some(Self::NiftyIt),
            "NIFTYPHARMA" => Some(Self::NiftyPharma),
            "NIFTYAUTO" => Some(Self::NiftyAuto),
            "NIFTYFMCG" => Some(Self::NiftyFmcg),
            "NIFTYMETAL" => Some(Self::NiftyMetal),
            "NIFTYREALTY" => Some(Self::NiftyRealty),
            "NIFTYENERGY" => Some(Self::NiftyEnergy),
            "NIFTYINFRA" => Some(Self::NiftyInfra),
            "NIFTYPSUBANK" => Some(Self::NiftyPsuBank),
            "NIFTYFINANCIALSERVICES" | "NIFTYFINSERVICE" | "NIFTYFINANCE" => {
                Some(Self::NiftyFinancialServices)
            }
            _ => None,
        }
    }

    /// Embedded seed JSON content bundled at compile time.
    pub fn bundled_seed_str(&self) -> &'static str {
        match self {
            Self::Nifty50 => include_str!("../../seeds/indices/nifty_50.json"),
            Self::NiftyNext50 => include_str!("../../seeds/indices/nifty_next_50.json"),
            Self::Nifty100 => include_str!("../../seeds/indices/nifty_100.json"),
            Self::Nifty200 => include_str!("../../seeds/indices/nifty_200.json"),
            Self::Nifty500 => include_str!("../../seeds/indices/nifty_500.json"),
            Self::NiftyMidcap50 => include_str!("../../seeds/indices/nifty_midcap_50.json"),
            Self::NiftyMidcap100 => include_str!("../../seeds/indices/nifty_midcap_100.json"),
            Self::NiftyMidcap150 => include_str!("../../seeds/indices/nifty_midcap_150.json"),
            Self::NiftySmallcap50 => include_str!("../../seeds/indices/nifty_smallcap_50.json"),
            Self::NiftySmallcap100 => include_str!("../../seeds/indices/nifty_smallcap_100.json"),
            Self::NiftySmallcap250 => include_str!("../../seeds/indices/nifty_smallcap_250.json"),
            Self::NiftyBank => include_str!("../../seeds/indices/nifty_bank.json"),
            Self::NiftyIt => include_str!("../../seeds/indices/nifty_it.json"),
            Self::NiftyPharma => include_str!("../../seeds/indices/nifty_pharma.json"),
            Self::NiftyAuto => include_str!("../../seeds/indices/nifty_auto.json"),
            Self::NiftyFmcg => include_str!("../../seeds/indices/nifty_fmcg.json"),
            Self::NiftyMetal => include_str!("../../seeds/indices/nifty_metal.json"),
            Self::NiftyRealty => include_str!("../../seeds/indices/nifty_realty.json"),
            Self::NiftyEnergy => include_str!("../../seeds/indices/nifty_energy.json"),
            Self::NiftyInfra => include_str!("../../seeds/indices/nifty_infra.json"),
            Self::NiftyPsuBank => include_str!("../../seeds/indices/nifty_psu_bank.json"),
            Self::NiftyFinancialServices => {
                include_str!("../../seeds/indices/nifty_financial_services.json")
            }
        }
    }
}

/// On-disk representation of an index constituent cache file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexCacheFile {
    pub alias: String,
    #[serde(alias = "last_updated")]
    pub as_of: String,
    #[serde(default)]
    pub source_url: Option<String>,
    pub symbols: Vec<String>,
}

/// In-memory entry holding constituent symbols and metadata for one index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub alias: IndexAlias,
    pub display_name: String,
    pub dsl_keyword: String,
    pub as_of: String,
    pub source_url: Option<String>,
    pub symbols: Vec<String>,
}

/// Lightweight metadata query result (zero disk or network I/O).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexMetadata {
    pub alias: String,
    pub display_name: String,
    pub dsl_keyword: String,
    pub symbol_count: usize,
    pub as_of: String,
}

/// Single in-memory registry of all 22 indices.
pub struct IndexRegistry {
    entries: RwLock<HashMap<IndexAlias, IndexEntry>>,
    cache_dir: Option<PathBuf>,
}

impl IndexRegistry {
    /// Loads the registry using the default cache directory (falling back to bundled seeds).
    pub fn new() -> Self {
        Self::load_with_cache_dir(Some(default_indices_cache_dir()))
    }

    /// Loads the registry from `cache_dir` if specified, falling back to bundled seeds.
    ///
    /// Cache in `cache_dir` wins over bundled seed files.
    pub fn load_with_cache_dir(cache_dir: Option<PathBuf>) -> Self {
        let mut map = HashMap::with_capacity(22);

        for alias in IndexAlias::all() {
            let loaded = cache_dir
                .as_ref()
                .and_then(|dir| Self::try_read_cache_file(dir, *alias))
                .unwrap_or_else(|| Self::read_seed_file(*alias));

            map.insert(
                *alias,
                IndexEntry {
                    alias: *alias,
                    display_name: alias.display_name().to_string(),
                    dsl_keyword: alias.dsl_keyword().to_string(),
                    as_of: loaded.as_of,
                    source_url: loaded.source_url,
                    symbols: loaded.symbols,
                },
            );
        }

        Self {
            entries: RwLock::new(map),
            cache_dir,
        }
    }

    fn try_read_cache_file(cache_dir: &Path, alias: IndexAlias) -> Option<IndexCacheFile> {
        let path = cache_dir.join(format!("{}.json", alias.alias()));
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    fn read_seed_file(alias: IndexAlias) -> IndexCacheFile {
        let text = alias.bundled_seed_str();
        serde_json::from_str(text).unwrap_or_else(|_| IndexCacheFile {
            alias: alias.dsl_keyword().to_string(),
            as_of: "never".to_string(),
            source_url: None,
            symbols: Vec::new(),
        })
    }

    /// Configured cache directory for this registry.
    pub fn cache_dir(&self) -> Option<&Path> {
        self.cache_dir.as_deref()
    }

    /// Read metadata for all 22 indices without disk or network I/O.
    pub fn list_metadata(&self) -> Vec<IndexMetadata> {
        let guard = self.entries.read().expect("registry lock poisoned");
        IndexAlias::all()
            .iter()
            .filter_map(|alias| guard.get(alias))
            .map(|entry| IndexMetadata {
                alias: entry.alias.alias().to_string(),
                display_name: entry.display_name.clone(),
                dsl_keyword: entry.dsl_keyword.clone(),
                symbol_count: entry.symbols.len(),
                as_of: entry.as_of.clone(),
            })
            .collect()
    }

    /// Returns constituent symbols for an index alias (zero I/O).
    pub fn get_symbols(&self, alias: IndexAlias) -> Vec<String> {
        let guard = self.entries.read().expect("registry lock poisoned");
        guard
            .get(&alias)
            .map(|e| e.symbols.clone())
            .unwrap_or_default()
    }

    /// Returns full entry for an index alias (zero I/O).
    pub fn get_entry(&self, alias: IndexAlias) -> Option<IndexEntry> {
        let guard = self.entries.read().expect("registry lock poisoned");
        guard.get(&alias).cloned()
    }

    /// Updates in-memory entry after a refresh or cache write.
    pub fn update_entry(&self, alias: IndexAlias, cache_file: IndexCacheFile) {
        let mut guard = self.entries.write().expect("registry lock poisoned");
        guard.insert(
            alias,
            IndexEntry {
                alias,
                display_name: alias.display_name().to_string(),
                dsl_keyword: alias.dsl_keyword().to_string(),
                as_of: cache_file.as_of,
                source_url: cache_file.source_url,
                symbols: cache_file.symbols,
            },
        );
    }
}

impl Default for IndexRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Default location for app-data cached indices.
pub fn default_indices_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ALGOMLN_INDICES_DIR") {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata).join("algomln").join("indices");
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local).join("algomln").join("indices");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
            return PathBuf::from(xdg).join("algomln").join("indices");
        }
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("share")
                .join("algomln")
                .join("indices");
        }
    }
    PathBuf::from("data").join("indices")
}
