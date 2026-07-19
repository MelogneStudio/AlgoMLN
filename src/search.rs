//! Pure fuzzy-search logic for the `search_symbols` IPC command.
//!
//! No Tauri imports here — this module is exercised directly by the
//! `search_symbols_impl` body in `src/commands/search.rs` and is unit-tested
//! in isolation.
//!
//! Scoring tiers (highest applicable tier wins; tiers are not additive):
//!
//! | Tier | Condition                                                | Score range     |
//! |------|----------------------------------------------------------|-----------------|
//! | 5    | candidate == query (exact)                               | 1000            |
//! | 4    | candidate.starts_with(query)                             | 800..900        |
//! | 3    | candidate.contains(query)                                | 500             |
//! | 2    | query is a subsequence of candidate                      | 300             |
//! | 1    | trigram Jaccard similarity >= 0.25                       | 50..200         |
//! | 0    | none of the above                                        | 0 (filtered)    |

use std::collections::{HashMap, HashSet};

use crate::indices::registry::IndexEntry;

/// Single search hit. `securityId` is `Some` for equities, `None` for indices.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolMatch {
    pub symbol: String,
    pub display_name: String,
    pub kind: SymbolKind,
    pub security_id: Option<u32>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SymbolKind {
    Equity,
    Index,
}

/// Score a candidate against a query. Both inputs are expected to already be
/// lowercased — see `fuzzy_search` which normalises its inputs.
///
/// Returns `0` if the score is below the relevance threshold.
pub fn score_match(query: &str, candidate: &str) -> u32 {
    // Tier 5 — exact match.
    if candidate == query {
        return 1000;
    }

    // Tier 4 — prefix match. Shorter symbols win because a query that
    // unambiguously names a 4-letter ticker should outrank the same prefix
    // embedded in a 30-character string.
    if let Some(rest) = candidate.strip_prefix(query) {
        // (100 - len.min(100)) rewards short candidates. `rest` excludes
        // the matched query so we add `query.len()` back to score against
        // the full candidate length.
        let total = rest.len() + query.len();
        let bonus = (100u32).saturating_sub(total.min(100) as u32);
        return 800 + bonus;
    }

    // Tier 3 — substring match.
    if candidate.contains(query) {
        return 500;
    }

    // Tier 2 — subsequence match (all query chars in order).
    if is_subsequence(query, candidate) {
        return 300;
    }

    // Tier 1 — trigram Jaccard.
    let sim = trigram_similarity(query, candidate);
    if sim >= 0.25 {
        let score = (sim * 200.0) as u32;
        return score.max(50);
    }

    0
}

/// True iff every char in `query` appears in `candidate` in the same order.
/// Standard O(|candidate|) two-pointer scan.
pub fn is_subsequence(query: &str, candidate: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let mut q = query.chars();
    let mut cur = q.next().expect("non-empty");
    for ch in candidate.chars() {
        if ch == cur {
            match q.next() {
                Some(next) => cur = next,
                None => return true,
            }
        }
    }
    false
}

/// Jaccard similarity over character-trigram sets. Returns `0.0` if either
/// string has fewer than 3 characters.
pub fn trigram_similarity(a: &str, b: &str) -> f32 {
    let set_a = trigrams(a);
    let set_b = trigrams(b);
    if set_a.is_empty() || set_b.is_empty() {
        return 0.0;
    }
    let intersection: usize = set_a.intersection(&set_b).count();
    let union: usize = set_a.union(&set_b).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f32 / union as f32
}

fn trigrams(s: &str) -> HashSet<String> {
    if s.chars().count() < 3 {
        return HashSet::new();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = HashSet::with_capacity(chars.len().saturating_sub(2));
    for window in chars.windows(3) {
        out.insert(window.iter().collect());
    }
    out
}

/// Fuzzy search over equities and indices. Returns up to `max_results`
/// sorted by descending score, with alphabetical `symbol` as the
/// tiebreaker so the order is deterministic across calls.
pub fn fuzzy_search(
    query: &str,
    equity_symbols: &HashMap<String, u32>,
    index_entries: &[IndexEntry],
    max_results: usize,
) -> Vec<SymbolMatch> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }

    let mut scored: Vec<(u32, SymbolMatch)> = Vec::new();

    // Equities.
    for (key, &sec_id) in equity_symbols {
        let candidate = key.to_lowercase();
        let score = score_match(&q, &candidate);
        if score > 0 {
            scored.push((
                score,
                SymbolMatch {
                    symbol: key.clone(),
                    display_name: key.clone(),
                    kind: SymbolKind::Equity,
                    security_id: Some(sec_id),
                },
            ));
        }
    }

    // Indices — try the display name AND the DSL-keyword form so a user
    // can search either by the human label ("nifty 50") or the keyword
    // ("nifty_50"). Keep the best of the two.
    for entry in index_entries {
        let name_lower = entry.display_name.to_lowercase();
        let keyword_lower = entry.alias.dsl_keyword().to_lowercase();
        let score_name = score_match(&q, &name_lower);
        let score_keyword = score_match(&q, &keyword_lower);
        let score = score_name.max(score_keyword);
        if score > 0 {
            scored.push((
                score,
                SymbolMatch {
                    symbol: entry.display_name.clone(),
                    display_name: entry.display_name.clone(),
                    kind: SymbolKind::Index,
                    security_id: None,
                },
            ));
        }
    }

    // Sort: score DESC, then symbol ASC for determinism.
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.symbol.cmp(&b.1.symbol)));

    scored.truncate(max_results);
    scored.into_iter().map(|(_, m)| m).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::dsl::ast::IndexAlias;

    fn entry(name: &str) -> IndexEntry {
        IndexEntry {
            alias: IndexAlias::Nifty50,
            display_name: name.to_string(),
            symbols: vec![],
            last_updated: "never".to_string(),
        }
    }

    #[test]
    fn tier_5_exact() {
        assert_eq!(score_match("reliance", "reliance"), 1000);
    }

    #[test]
    fn tier_4_prefix_prefers_shorter() {
        // Same prefix; shorter candidate should score higher. The
        // length bonus is `(100 - candidate_len())`, so 4-char and
        // 5-char candidates differ by 1.
        let four = score_match("rel", "rela");
        let five = score_match("rel", "relat");
        assert!(four > five);
        // 4 chars → 800 + 96 = 896
        assert_eq!(four, 800 + (100 - 4));
    }

    #[test]
    fn tier_5_beats_tier_4() {
        // Exact match (1000) outranks any prefix score (800..900).
        assert!(score_match("rel", "rel") > score_match("rel", "relx"));
    }

    #[test]
    fn tier_3_contains() {
        assert_eq!(score_match("iance", "reliance"), 500);
    }

    #[test]
    fn tier_2_subsequence() {
        // r..e..l: r-e-l appear in order in "reliance".
        assert_eq!(score_match("rln", "reliance"), 300);
    }

    #[test]
    fn tier_1_trigram_below_threshold_returns_zero() {
        // Two very dissimilar strings should fail all tiers.
        assert_eq!(score_match("xyz", "aaaa"), 0);
    }

    #[test]
    fn tier_1_trigram_above_threshold() {
        // Construct a query that has high trigram overlap with the
        // candidate but breaks subsequence matching by duplicating a
        // char. "bankofiindia" vs "bankofindia":
        //   - not equal, not prefix, not contains
        //   - subsequence: b-a-n-k-o-f-i-i-n-d-i-a — the second `i`
        //     is at position 9, but the next needed `n` doesn't exist
        //     after position 9 in "bankofindia" (ends with ...i-a).
        //     So subsequence is false.
        //   - trigrams: 8 of the 10 query trigrams are also in the
        //     candidate, Jaccard ≈ 0.73 → well above the 0.25 floor.
        let s = score_match("bankofiindia", "bankofindia");
        assert!(s >= 50 && s <= 200, "expected 50..200, got {}", s);
    }

    #[test]
    fn is_subsequence_works() {
        assert!(is_subsequence("rln", "reliance"));
        assert!(is_subsequence("ab", "abcdef"));
        assert!(!is_subsequence("abc", "acb"));
        assert!(is_subsequence("", "anything"));
    }

    #[test]
    fn trigram_similarity_short_input() {
        assert_eq!(trigram_similarity("ab", "abcdef"), 0.0);
    }

    #[test]
    fn trigram_similarity_identical() {
        assert!((trigram_similarity("reliance", "reliance") - 1.0).abs() < 1e-6);
    }

    #[test]
    fn empty_query_returns_empty() {
        let mut eq = HashMap::new();
        eq.insert("RELIANCE".to_string(), 2885u32);
        assert!(fuzzy_search("", &eq, &[], 5).is_empty());
        assert!(fuzzy_search("   ", &eq, &[], 5).is_empty());
    }

    #[test]
    fn equity_hit() {
        let mut eq = HashMap::new();
        eq.insert("RELIANCE".to_string(), 2885u32);
        eq.insert("HDFCBANK".to_string(), 1333u32);
        let r = fuzzy_search("rel", &eq, &[], 5);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].symbol, "RELIANCE");
        assert_eq!(r[0].security_id, Some(2885));
        assert!(matches!(r[0].kind, SymbolKind::Equity));
    }

    #[test]
    fn index_hit_uses_display_name() {
        let mut eq = HashMap::new();
        let entries = vec![entry("NIFTY 50")];
        let r = fuzzy_search("nifty 50", &eq, &entries, 5);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].symbol, "NIFTY 50");
        assert_eq!(r[0].security_id, None);
        assert!(matches!(r[0].kind, SymbolKind::Index));
    }

    #[test]
    fn index_hit_uses_keyword() {
        let mut eq = HashMap::new();
        let entries = vec![entry("NIFTY 50")];
        let r = fuzzy_search("nifty_50", &eq, &entries, 5);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn max_results_truncates() {
        let mut eq = HashMap::new();
        for i in 0..20 {
            eq.insert(format!("ABC{}", i), i as u32);
        }
        let r = fuzzy_search("abc", &eq, &[], 3);
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn ranking_orders_higher_score_first() {
        let mut eq = HashMap::new();
        // Exact > prefix > contains > subsequence.
        eq.insert("RELIANCE".to_string(), 1u32);
        eq.insert("REL".to_string(), 2u32);
        eq.insert("ASDFRELIANCE".to_string(), 3u32);
        let r = fuzzy_search("rel", &eq, &[], 5);
        assert_eq!(r[0].symbol, "REL"); // exact
        assert_eq!(r[1].symbol, "RELIANCE"); // prefix
                                             // "ASDFRELIANCE" is a contains match (score 500) and should
                                             // rank below the prefix match (score >= 800).
        assert_eq!(r[2].symbol, "ASDFRELIANCE");
    }
}
