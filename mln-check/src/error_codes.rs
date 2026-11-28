use mln_lang::ErrorCodeInfo;

/// Diagnostic codes emitted by Phase 2 (`mln-check`).
pub const PHASE2_CODES: &[ErrorCodeInfo] = &[
    ErrorCodeInfo {
        code: "E0200",
        phase: 2,
        summary: "unknown identifier",
    },
    ErrorCodeInfo {
        code: "E0201",
        phase: 2,
        summary: "redefinition of name",
    },
    ErrorCodeInfo {
        code: "E0202",
        phase: 2,
        summary: "use before definition",
    },
    ErrorCodeInfo {
        code: "E0203",
        phase: 2,
        summary: "shadowing a builtin",
    },
    ErrorCodeInfo {
        code: "E0210",
        phase: 2,
        summary: "recursive def",
    },
    ErrorCodeInfo {
        code: "E0220",
        phase: 2,
        summary: "missing entry or exit output",
    },
    ErrorCodeInfo {
        code: "E0230",
        phase: 2,
        summary: "invalid state assignment target",
    },
    ErrorCodeInfo {
        code: "W0201",
        phase: 2,
        summary: "unused name",
    },
    ErrorCodeInfo {
        code: "E0300",
        phase: 2,
        summary: "type mismatch",
    },
    ErrorCodeInfo {
        code: "E0301",
        phase: 2,
        summary: "wrong call arity",
    },
    ErrorCodeInfo {
        code: "E0302",
        phase: 2,
        summary: "unknown keyword argument",
    },
    ErrorCodeInfo {
        code: "E0303",
        phase: 2,
        summary: "argument type mismatch",
    },
    ErrorCodeInfo {
        code: "E0310",
        phase: 2,
        summary: "negative index",
    },
    ErrorCodeInfo {
        code: "E0311",
        phase: 2,
        summary: "non-constant index",
    },
    ErrorCodeInfo {
        code: "E0401",
        phase: 2,
        summary: "finer clock inside coarser clock",
    },
    ErrorCodeInfo {
        code: "E0402",
        phase: 2,
        summary: "@ applied to constant",
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn phase2_codes_unique_and_valid() {
        let mut seen = HashSet::new();
        for entry in PHASE2_CODES {
            assert!(seen.insert(entry.code), "duplicate code {}", entry.code);
            let bytes = entry.code.as_bytes();
            assert_eq!(bytes.len(), 5);
            assert!(bytes[0] == b'E' || bytes[0] == b'W');
            let n: u32 = entry.code[1..].parse().unwrap();
            assert!((200..=499).contains(&n), "unexpected code {}", entry.code);
        }
    }
}
