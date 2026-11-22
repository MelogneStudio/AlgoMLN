pub const MAX_CORRELATION_LEN: usize = 30;
pub const DEFAULT_PREFIX: &str = "AMLN";

/// Generates deterministic, unique, Dhan-compliant correlation IDs.
/// Format: ASCII letters, digits, underscore and hyphen, at most 30 characters.
/// e.g. "AMLN-R1-000001"
#[derive(Clone, Debug)]
pub struct CorrelationGenerator {
    prefix: String,
    run_tag: String,
}

impl CorrelationGenerator {
    pub fn new(run_tag: &str) -> Self {
        Self::with_prefix(DEFAULT_PREFIX, run_tag)
    }

    pub fn with_prefix(prefix: &str, run_tag: &str) -> Self {
        // Sanitize prefix and run_tag to only valid ASCII letters, digits, _ and -
        let clean_prefix: String = prefix
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        let clean_tag: String = run_tag
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();

        // Calculate available room: prefix + "-" + tag + "-" + sequence (up to 7 digits) <= 30
        let base_room = MAX_CORRELATION_LEN.saturating_sub(clean_prefix.len() + 2 + 7);
        let tag_truncated: String = clean_tag.chars().take(base_room.max(1)).collect();

        Self {
            prefix: clean_prefix,
            run_tag: tag_truncated,
        }
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub fn generate(&self, seq: u64) -> String {
        let cid = format!("{}-{}-{:04}", self.prefix, self.run_tag, seq);
        if cid.len() > MAX_CORRELATION_LEN {
            cid.chars().take(MAX_CORRELATION_LEN).collect()
        } else {
            cid
        }
    }

    /// Check if correlation ID carries our AlgoMLN ownership prefix.
    pub fn is_own(&self, correlation_id: &str) -> bool {
        let trimmed = correlation_id.trim();
        trimmed.starts_with(&self.prefix)
            || trimmed.starts_with("AlgoMLN")
            || trimmed.starts_with("AMLN")
    }
}

pub fn is_valid_correlation_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_CORRELATION_LEN
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}
