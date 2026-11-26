use std::collections::HashSet;

use mln_lang::{dump, lex, line_col, parse, render, ERROR_CODES};

#[test]
fn d02_codes_unique_and_phase_range() {
    let mut seen = HashSet::new();
    for entry in ERROR_CODES {
        assert!(seen.insert(entry.code), "duplicate code {}", entry.code);
        let bytes = entry.code.as_bytes();
        assert_eq!(bytes.len(), 5);
        assert!(bytes[0] == b'E' || bytes[0] == b'W');
        assert!(bytes[1..].iter().all(|b| b.is_ascii_digit()));
        let n: u32 = entry.code[1..].parse().unwrap();
        assert!(n <= 199, "Phase 1 code {} out of range", entry.code);
    }
}

#[test]
fn d03_diagnostics_sorted_and_deterministic() {
    let src = "x = a % b\ny += 1\n1 = z\nx = @\n";
    let out1 = parse(src);
    let out2 = parse(src);
    assert_eq!(out1.diags, out2.diags);
    assert_eq!(dump(&out1.program), dump(&out2.program));

    for w in out1.diags.windows(2) {
        let ord = w[0]
            .span
            .start
            .cmp(&w[1].span.start)
            .then_with(|| w[0].code.cmp(w[1].code));
        assert!(ord.is_le(), "diagnostics not sorted: {:?}", out1.diags);
    }
}

#[test]
fn d04_spans_within_bounds_and_char_boundaries() {
    let cases = [
        "x = é\ny = (1\n",
        "# é\r\na = 1.\n'unterminated\n",
        "def f(x):\n\treturn x\n",
        &"x".repeat(1024 * 1024 + 5),
    ];
    for src in cases {
        let out = parse(src);
        for d in &out.diags {
            assert!(
                d.span.start <= d.span.end,
                "invalid span order {:?} in {src:?}",
                d.span
            );
            assert!(
                (d.span.end as usize) <= src.len(),
                "span out of bounds {:?} > {} in {src:?}",
                d.span,
                src.len()
            );
            assert!(
                src.is_char_boundary(d.span.start as usize),
                "start not on char boundary"
            );
            assert!(
                src.is_char_boundary(d.span.end as usize),
                "end not on char boundary"
            );
        }
    }
}

#[test]
fn d05_help_present_on_required_codes() {
    let cases = [
        ("x = a && b", "E0006"),
        ("for x in y", "E0021"),
        ("x = 1; y = 2", "E0130"),
        ("ema(close, 5)", "E0152"),
        ("x += 1", "E0154"),
        ("a % b", "E0001"),
    ];
    for (src, code) in cases {
        let out = parse(src);
        let d = out
            .diags
            .iter()
            .find(|d| d.code == code)
            .unwrap_or_else(|| panic!("expected {code} for {src:?}, got {:?}", out.diags));
        assert!(
            d.help.as_ref().is_some_and(|h| !h.is_empty()),
            "expected non-empty help for {code} on {src:?}"
        );
    }
}

#[test]
fn d06_200_lines_of_garbage_caps_at_50_plus_e0199() {
    let src = (0..200).map(|_| "1 = x").collect::<Vec<_>>().join("\n");
    let out = parse(&src);
    assert_eq!(out.diags.len(), 51);
    assert!(out.diags[..50].iter().all(|d| d.code != "E0199"));
    assert_eq!(out.diags[50].code, "E0199");
}

#[test]
fn d07_independent_errors_on_separate_lines() {
    let out = parse("1 = x\nx += 1\nema(close, 5)\nreturn 1");
    let codes: Vec<&str> = out.diags.iter().map(|d| d.code).collect();
    assert_eq!(codes, vec!["E0153", "E0154", "E0152", "E0150"]);
}

#[test]
fn d08_every_lex_diag_appears_in_parse() {
    let cases = [
        "x = 1.\ny = 5min\nz = 0m",
        "x = a && b\ny = '\\q'\nz = é",
        "def f(x):\n\treturn x\ny = (1",
        "for x in y\nz = 1)",
    ];
    for src in cases {
        let lex_diags = lex(src).diags;
        let parse_diags = parse(src).diags;
        for ld in &lex_diags {
            assert!(
                parse_diags.contains(ld),
                "lex diagnostic {ld:?} missing from parse diagnostics {parse_diags:?} for {src:?}"
            );
        }
    }
}

#[test]
fn d09_line_col_1_based_chars_and_crlf() {
    let src = "# café\r\nx = 1";
    // Line 1: "# café" -> '#'(0) ' '(1) 'c'(2) 'a'(3) 'f'(4) 'é'(5..7) '\r'(7) '\n'(8)
    // Line 2: "x = 1" starts at byte 9
    assert_eq!(line_col(src, 0), (1, 1));
    assert_eq!(line_col(src, 5), (1, 6));
    assert_eq!(line_col(src, 7), (1, 7));
    assert_eq!(line_col(src, 9), (2, 1));
    assert_eq!(line_col(src, 13), (2, 5));
}

#[test]
fn d10_render_prints_line_col_and_caret() {
    let src = "x = a && b\n";
    let out = parse(src);
    let rendered = render(&out.diags[0], src);
    assert!(rendered.contains("E0006"), "{rendered}");
    assert!(rendered.contains("1:7"), "{rendered}");
    assert!(rendered.contains("x = a && b"), "{rendered}");
    assert!(rendered.contains("      ^^"), "{rendered}");
    assert!(rendered.contains("help:"), "{rendered}");
}
