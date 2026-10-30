use std::{fs, path::Path};

#[test]
fn golden_strategy_samples_match_serialized_ast() {
    for name in ["trend_momentum", "mean_reversion"] {
        let source = fs::read_to_string(format!("samples/{name}.algomln")).unwrap();
        let expected: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(format!("tests/golden/{name}.ast.json")).unwrap(),
        )
        .unwrap();
        let actual = algomln::compile(&source).unwrap();
        assert_eq!(
            serde_json::to_value(actual.strategy()).unwrap(),
            expected,
            "{name}"
        );
    }
}

#[test]
fn golden_error_cases_match_stable_codes() {
    let root = Path::new("tests/golden");
    let name = "malformed";
    let path = root.join(format!("{name}.algomln"));
    let source = fs::read_to_string(&path).unwrap();
    let expected = fs::read_to_string(root.join(format!("{name}.error"))).unwrap();
    let error = algomln::compile(&source).unwrap_err();
    assert_eq!(error.code(), expected.trim(), "{}", path.display());
}
