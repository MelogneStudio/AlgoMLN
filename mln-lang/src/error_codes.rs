/// Metadata for a registered diagnostic code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ErrorCodeInfo {
    /// Stable diagnostic code (e.g. `"E0001"`).
    pub code: &'static str,
    /// Compiler phase where this code originates.
    pub phase: u8,
    /// Short summary matching the specification.
    pub summary: &'static str,
}

/// Registry of all diagnostic codes defined for Phase 1.
pub const ERROR_CODES: &[ErrorCodeInfo] = &[
    // Lexer: E00xx
    ErrorCodeInfo {
        code: "E0001",
        phase: 1,
        summary: "invalid character",
    },
    ErrorCodeInfo {
        code: "E0002",
        phase: 1,
        summary: "unterminated string",
    },
    ErrorCodeInfo {
        code: "E0003",
        phase: 1,
        summary: "malformed number",
    },
    ErrorCodeInfo {
        code: "E0004",
        phase: 1,
        summary: "unknown number suffix",
    },
    ErrorCodeInfo {
        code: "E0005",
        phase: 1,
        summary: "invalid timeframe literal",
    },
    ErrorCodeInfo {
        code: "E0006",
        phase: 1,
        summary: "unsupported operator symbol (`&&`, `||`, `!`)",
    },
    ErrorCodeInfo {
        code: "E0007",
        phase: 1,
        summary: "invalid string escape",
    },
    ErrorCodeInfo {
        code: "E0010",
        phase: 1,
        summary: "tab in indentation",
    },
    ErrorCodeInfo {
        code: "E0011",
        phase: 1,
        summary: "indent jumps more than one level",
    },
    ErrorCodeInfo {
        code: "E0012",
        phase: 1,
        summary: "indent is not a multiple of 4 spaces",
    },
    ErrorCodeInfo {
        code: "E0013",
        phase: 1,
        summary: "indented first line",
    },
    ErrorCodeInfo {
        code: "E0014",
        phase: 1,
        summary: "unmatched closing bracket",
    },
    ErrorCodeInfo {
        code: "E0015",
        phase: 1,
        summary: "unclosed bracket at end of file",
    },
    ErrorCodeInfo {
        code: "E0017",
        phase: 1,
        summary: "source too large",
    },
    ErrorCodeInfo {
        code: "E0021",
        phase: 1,
        summary: "unsupported Python keyword",
    },
    // Parser: E01xx
    ErrorCodeInfo {
        code: "E0100",
        phase: 1,
        summary: "unexpected token",
    },
    ErrorCodeInfo {
        code: "E0101",
        phase: 1,
        summary: "expected expression",
    },
    ErrorCodeInfo {
        code: "E0102",
        phase: 1,
        summary: "expected identifier",
    },
    ErrorCodeInfo {
        code: "E0103",
        phase: 1,
        summary: "expected closing bracket",
    },
    ErrorCodeInfo {
        code: "E0110",
        phase: 1,
        summary: "positional argument after keyword argument",
    },
    ErrorCodeInfo {
        code: "E0111",
        phase: 1,
        summary: "duplicate keyword argument",
    },
    ErrorCodeInfo {
        code: "E0112",
        phase: 1,
        summary: "empty index",
    },
    ErrorCodeInfo {
        code: "E0113",
        phase: 1,
        summary: "slices are not supported",
    },
    ErrorCodeInfo {
        code: "E0114",
        phase: 1,
        summary: "expected timeframe after `@`",
    },
    ErrorCodeInfo {
        code: "E0115",
        phase: 1,
        summary: "duplicate parameter",
    },
    ErrorCodeInfo {
        code: "E0116",
        phase: 1,
        summary: "required parameter after a default parameter",
    },
    ErrorCodeInfo {
        code: "E0117",
        phase: 1,
        summary: "nested def",
    },
    ErrorCodeInfo {
        code: "E0118",
        phase: 1,
        summary: "def block missing its final return",
    },
    ErrorCodeInfo {
        code: "E0119",
        phase: 1,
        summary: "only assignments allowed in a def block",
    },
    ErrorCodeInfo {
        code: "E0120",
        phase: 1,
        summary: "nesting too deep",
    },
    ErrorCodeInfo {
        code: "E0130",
        phase: 1,
        summary: "`;` is not supported",
    },
    ErrorCodeInfo {
        code: "E0140",
        phase: 1,
        summary: "block not available in this version",
    },
    ErrorCodeInfo {
        code: "E0150",
        phase: 1,
        summary: "return outside a def, or not last",
    },
    ErrorCodeInfo {
        code: "E0152",
        phase: 1,
        summary: "expression statement has no effect",
    },
    ErrorCodeInfo {
        code: "E0153",
        phase: 1,
        summary: "invalid assignment target",
    },
    ErrorCodeInfo {
        code: "E0154",
        phase: 1,
        summary: "augmented assignment is not supported",
    },
    ErrorCodeInfo {
        code: "E0155",
        phase: 1,
        summary: "unexpected indent",
    },
    ErrorCodeInfo {
        code: "E0199",
        phase: 1,
        summary: "too many errors",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn registry_codes_are_unique_and_well_formed() {
        let mut seen = HashSet::new();
        for entry in ERROR_CODES {
            assert!(
                seen.insert(entry.code),
                "duplicate error code in registry: {}",
                entry.code
            );
            let bytes = entry.code.as_bytes();
            assert_eq!(bytes.len(), 5, "bad code length: {}", entry.code);
            assert!(
                bytes[0] == b'E' || bytes[0] == b'W',
                "code must start with E or W: {}",
                entry.code
            );
            assert!(
                bytes[1..].iter().all(|b| b.is_ascii_digit()),
                "code must end with 4 digits: {}",
                entry.code
            );
            if entry.phase == 1 {
                let num: u32 = entry.code[1..].parse().expect("4 digits");
                assert!(
                    num <= 199,
                    "Phase 1 code {} outside E00xx/E01xx range",
                    entry.code
                );
            }
        }
    }
}
