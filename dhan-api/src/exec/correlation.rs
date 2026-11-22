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
        let seq_str = format!("{:04}", seq);
        let max_base_len = MAX_CORRELATION_LEN.saturating_sub(seq_str.len() + 1);
        let mut base = format!("{}-{}", self.prefix, self.run_tag);
        if base.len() > max_base_len {
            base.truncate(max_base_len);
        }
        format!("{}-{}", base, seq_str)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_correlation_preserves_sequence_suffix_on_truncation() {
        let gen = CorrelationGenerator::with_prefix("AMLN", "SUPER_LONG_RUN_TAG_NAME_THAT_EXCEEDS_LIMIT");
        let cid = gen.generate(12345);
        assert!(is_valid_correlation_id(&cid));
        assert!(cid.ends_with("-12345"), "CID should end with -12345 but was {}", cid);
        assert!(cid.len() <= MAX_CORRELATION_LEN);
    }
}
