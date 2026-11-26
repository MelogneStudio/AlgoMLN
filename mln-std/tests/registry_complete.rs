use std::path::Path;

use mln_std::TIER1_OPS;

#[test]
fn t3_04_registry_completeness_and_fixtures_exist() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    for op in TIER1_OPS {
        assert!(!op.name.is_empty());
        assert!(
            !op.doc_example.is_empty(),
            "missing doc_example for {}",
            op.name
        );
        assert!(
            !op.fixture_path.is_empty(),
            "missing fixture_path for {}",
            op.name
        );
        let full = crate_dir.join(op.fixture_path);
        assert!(
            full.exists(),
            "fixture missing for {}: {}",
            op.name,
            full.display()
        );
        let contents = std::fs::read_to_string(&full).expect("read fixture");
        assert!(
            contents.lines().count() >= 5001,
            "fixture {} should have header + 5000 rows",
            full.display()
        );
    }
}
