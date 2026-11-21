use std::collections::{BTreeMap, HashMap};

use algomln::ast::TradeIn;
use dhan_api::Scrip;
use serde::{Deserialize, Serialize};

use crate::catalog::Catalog;
use crate::indices::registry::{IndexAlias, IndexRegistry};

pub const ERR_UNKNOWN_INDEX: &str = "ERR_UNKNOWN_INDEX";
pub const ERR_UNKNOWN_SYMBOL: &str = "ERR_UNKNOWN_SYMBOL";
pub const ERR_UNRESOLVED_CONSTITUENT: &str = "ERR_UNRESOLVED_CONSTITUENT";

/// Resolved equity instrument ready for engine and candle streaming.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedSymbol {
    pub security_id: String,
    pub symbol: String,
    pub lot_size: u32,
    pub tick_size_paise: i64,
    pub is_fno: bool,
}

/// Report for one expanded index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexResolutionReport {
    pub alias: String,
    pub display_name: String,
    pub as_of: String,
    pub constituent_count: usize,
    pub resolved_count: usize,
    pub unresolved_symbols: Vec<String>,
}

/// Full resolution report returned alongside resolved symbols.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolutionReport {
    pub indices: Vec<IndexResolutionReport>,
    pub total_resolved: usize,
    pub allow_unresolved: bool,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ResolveError {
    #[error("[{code}] unknown index '{name}'")]
    UnknownIndex {
        code: &'static str,
        name: String,
    },

    #[error("[{code}] unknown symbol '{name}'{hint}")]
    UnknownSymbol {
        code: &'static str,
        name: String,
        hint: String,
    },

    #[error(
        "[{code}] index '{index}' has {count} unresolved constituent(s): {symbols:?}; re-run with --allow-unresolved to skip them"
    )]
    UnresolvedConstituents {
        code: &'static str,
        index: String,
        count: usize,
        symbols: Vec<String>,
    },
}

/// Formats the user-facing run fingerprint combining the dataset content hash
/// with the as_of dates of any used indices.
pub fn format_run_fingerprint(
    dataset_fp: &algomln::sim::Fingerprint,
    report: &ResolutionReport,
) -> String {
    if report.indices.is_empty() {
        format!("{dataset_fp}")
    } else {
        let indices_part = report
            .indices
            .iter()
            .map(|idx| format!("{}@{}", idx.alias, idx.as_of))
            .collect::<Vec<_>>()
            .join(",");
        format!("{dataset_fp} [{indices_part}]")
    }
}

/// Resolves `TRADE_IN` AST node (either `TradeIn::Index` or `TradeIn::Symbols`)
/// against an index registry and scrip master (via `Catalog` or scrips slice).
pub struct TradeInResolver<'a> {
    registry: &'a IndexRegistry,
}

impl<'a> TradeInResolver<'a> {
    pub fn new(registry: &'a IndexRegistry) -> Self {
        Self { registry }
    }

    /// Resolves against SQLite catalog `instruments` table.
    pub fn resolve_catalog(
        &self,
        trade_in: &TradeIn,
        catalog: &Catalog,
        allow_unresolved: bool,
    ) -> Result<(Vec<ResolvedSymbol>, ResolutionReport), ResolveError> {
        let lookup = |sym: &str| -> Option<ResolvedSymbol> {
            catalog.resolve_full(sym).ok()
        };
        self.resolve_internal(trade_in, lookup, allow_unresolved)
    }

    /// Resolves against in-memory `Scrip` rows (parsed from Dhan scrip master).
    pub fn resolve_scrips(
        &self,
        trade_in: &TradeIn,
        scrips: &[Scrip],
        allow_unresolved: bool,
    ) -> Result<(Vec<ResolvedSymbol>, ResolutionReport), ResolveError> {
        let mut map: HashMap<String, ResolvedSymbol> = HashMap::with_capacity(scrips.len());
        for s in scrips {
            map.insert(
                s.symbol.to_ascii_uppercase(),
                ResolvedSymbol {
                    security_id: s.security_id.clone(),
                    symbol: s.symbol.to_ascii_uppercase(),
                    lot_size: s.lot_size,
                    tick_size_paise: s.tick_size_paise,
                    is_fno: s.is_fno,
                },
            );
        }
        let lookup = |sym: &str| -> Option<ResolvedSymbol> {
            map.get(&sym.to_ascii_uppercase()).cloned()
        };
        self.resolve_internal(trade_in, lookup, allow_unresolved)
    }

    fn resolve_internal<F>(
        &self,
        trade_in: &TradeIn,
        lookup: F,
        allow_unresolved: bool,
    ) -> Result<(Vec<ResolvedSymbol>, ResolutionReport), ResolveError>
    where
        F: Fn(&str) -> Option<ResolvedSymbol>,
    {
        match trade_in {
            TradeIn::Index(index_name) => {
                let alias = IndexAlias::from_keyword(index_name).ok_or_else(|| {
                    ResolveError::UnknownIndex {
                        code: ERR_UNKNOWN_INDEX,
                        name: index_name.clone(),
                    }
                })?;

                let entry = self.registry.get_entry(alias).ok_or_else(|| {
                    ResolveError::UnknownIndex {
                        code: ERR_UNKNOWN_INDEX,
                        name: index_name.clone(),
                    }
                })?;

                let mut resolved_map = BTreeMap::<String, ResolvedSymbol>::new();
                let mut unresolved = Vec::new();

                for sym in &entry.symbols {
                    if let Some(r) = lookup(sym) {
                        resolved_map.insert(r.symbol.clone(), r);
                    } else if !unresolved.contains(sym) {
                        unresolved.push(sym.clone());
                    }
                }

                if !unresolved.is_empty() && !allow_unresolved {
                    return Err(ResolveError::UnresolvedConstituents {
                        code: ERR_UNRESOLVED_CONSTITUENT,
                        index: alias.dsl_keyword().to_string(),
                        count: unresolved.len(),
                        symbols: unresolved,
                    });
                }

                let resolved: Vec<ResolvedSymbol> = resolved_map.into_values().collect();
                let report = ResolutionReport {
                    indices: vec![IndexResolutionReport {
                        alias: alias.dsl_keyword().to_string(),
                        display_name: alias.display_name().to_string(),
                        as_of: entry.as_of.clone(),
                        constituent_count: entry.symbols.len(),
                        resolved_count: resolved.len(),
                        unresolved_symbols: unresolved,
                    }],
                    total_resolved: resolved.len(),
                    allow_unresolved,
                };

                Ok((resolved, report))
            }
            TradeIn::Symbols(symbols) => {
                let mut resolved_map = BTreeMap::<String, ResolvedSymbol>::new();

                for sym in symbols {
                    let trimmed = sym.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Some(r) = lookup(trimmed) {
                        resolved_map.insert(r.symbol.clone(), r);
                    } else {
                        // Check if user specified an index name as a bare symbol
                        let hint = if let Some(alias) = IndexAlias::from_keyword(trimmed) {
                            format!(" (did you mean 'INDEX {}'?)", alias.dsl_keyword())
                        } else {
                            String::new()
                        };
                        return Err(ResolveError::UnknownSymbol {
                            code: ERR_UNKNOWN_SYMBOL,
                            name: trimmed.to_string(),
                            hint,
                        });
                    }
                }

                let resolved: Vec<ResolvedSymbol> = resolved_map.into_values().collect();
                let report = ResolutionReport {
                    indices: Vec::new(),
                    total_resolved: resolved.len(),
                    allow_unresolved,
                };

                Ok((resolved, report))
            }
        }
    }
}
