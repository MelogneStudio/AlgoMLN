use std::fs;
use std::path::{Path, PathBuf};

use mln_lang::{dump, line_col, parse, Diagnostic, ERROR_CODES};

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn format_diags(src: &str, diags: &[Diagnostic]) -> String {
    diags
        .iter()
        .map(|d| {
            let (line, col) = line_col(src, d.span.start);
            format!("{} {}:{} {}", d.code, line, col, d.message)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn check_ok_fixture(name: &str) {
    let ok_dir = fixtures_dir().join("ok");
    let src = fs::read_to_string(ok_dir.join(format!("{name}.mln"))).expect("read .mln");
    let expected = fs::read_to_string(ok_dir.join(format!("{name}.sexpr")))
        .expect("read .sexpr")
        .replace("\r\n", "\n");
    let out = parse(&src);
    assert!(
        out.diags.is_empty(),
        "expected no diagnostics for {name}.mln, got {:?}",
        out.diags
    );
    let dumped = dump(&out.program);
    assert_eq!(
        dumped,
        expected.trim_end_matches('\n'),
        "mismatch for {name}"
    );
    insta::assert_snapshot!(format!("ok_{name}"), dumped);
}

#[test]
fn g01_trend() {
    check_ok_fixture("trend");
}

#[test]
fn g02_multi_timeframe() {
    check_ok_fixture("multi_timeframe");
}

#[test]
fn g03_def_block() {
    check_ok_fixture("def_block");
}

#[test]
fn g04_percent_and_tf() {
    check_ok_fixture("percent_and_tf");
}

#[test]
fn g05_comments_blank_lines() {
    check_ok_fixture("comments_blank_lines");
}

#[test]
fn g06_crlf() {
    let ok_dir = fixtures_dir().join("ok");
    let raw = fs::read_to_string(ok_dir.join("crlf.mln")).expect("read crlf.mln");
    assert!(raw.contains("\r\n"), "crlf.mln must contain CRLF bytes");
    check_ok_fixture("crlf");
}

#[test]
fn g07_vcp_snippet() {
    check_ok_fixture("vcp_snippet");
}

#[test]
fn g08_unicode_in_comments_and_strings() {
    check_ok_fixture("unicode_in_comments_and_strings");
}

#[test]
fn d01_every_error_code_has_doc_and_fixture() {
    let docs_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("docs")
        .join("errors.md");
    let docs = fs::read_to_string(&docs_path).expect("read docs/errors.md");

    let err_dir = fixtures_dir().join("err");
    let err_files: Vec<PathBuf> = fs::read_dir(&err_dir)
        .expect("read err dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("mln"))
        .collect();

    for entry in ERROR_CODES {
        assert!(
            docs.contains(entry.code),
            "docs/errors.md is missing entry for {}",
            entry.code
        );

        let prefix = format!("{}_", entry.code);
        let matching: Vec<&PathBuf> = err_files
            .iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix))
            })
            .collect();
        assert!(
            !matching.is_empty(),
            "missing fixture tests/fixtures/err/{}*.mln for code {}",
            prefix,
            entry.code
        );

        for path in matching {
            let src = fs::read_to_string(path).expect("read err fixture");
            let out = parse(&src);
            assert!(
                out.diags.iter().any(|d| d.code == entry.code),
                "fixture {} did not trigger {}",
                path.display(),
                entry.code
            );
        }
    }
}

#[test]
fn error_fixture_diag_goldens() {
    let err_dir = fixtures_dir().join("err");
    let mut err_files: Vec<PathBuf> = fs::read_dir(&err_dir)
        .expect("read err dir")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("mln"))
        .collect();
    err_files.sort();

    for path in err_files {
        let diag_path = path.with_extension("diag");
        let src = fs::read_to_string(&path).expect("read err .mln");
        let out = parse(&src);
        let actual = format_diags(&src, &out.diags);
        let expected = fs::read_to_string(&diag_path)
            .unwrap_or_else(|e| panic!("missing {}: {e}", diag_path.display()))
            .replace("\r\n", "\n");
        assert_eq!(
            actual,
            expected.trim_end_matches('\n'),
            "diag mismatch for {}",
            path.display()
        );
    }
}
