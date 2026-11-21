//! Fixture-behavior tests keyed to #7's artifacts: every gold record under
//! `data/gold/` passes every check, and every near-miss record named in
//! `fixtures/near-miss/manifest.json` fails with the check matching its
//! planted defect. Until #7 lands, both tests skip with a message rather
//! than fabricating records.

use std::path::{Path, PathBuf};

use medsynth_validate::{
    Check, RecordFile, Validators, format_of, json_files, load_paths, run_checks,
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .to_path_buf()
}

#[test]
fn gold_set_passes_every_check() {
    let root = workspace_root();
    let gold = root.join("data/gold");
    if !gold.is_dir() {
        eprintln!("SKIP: data/gold/ does not exist yet — lands with #7");
        return;
    }
    let paths = json_files(&gold);
    assert!(!paths.is_empty(), "data/gold/ must contain records");
    let root2 = root.clone();
    let (records, load_findings) = load_paths(paths, move |p| {
        p.strip_prefix(&root2).unwrap_or(p).to_path_buf()
    });
    let mut findings = load_findings;
    findings.extend(run_checks(&records, &Validators::compile()));
    assert!(
        findings.is_empty(),
        "gold set must pass every check, got: {findings:#?}"
    );
}

#[test]
fn near_miss_fixtures_fail_for_their_planted_reason() {
    let root = workspace_root();
    let manifest_path = root.join("fixtures/near-miss/manifest.json");
    let Ok(text) = std::fs::read_to_string(&manifest_path) else {
        eprintln!("SKIP: fixtures/near-miss/manifest.json does not exist yet — lands with #7");
        return;
    };
    let manifest: serde_json::Value =
        serde_json::from_str(&text).expect("near-miss manifest parses");
    let entries = manifest["records"]
        .as_array()
        .expect("manifest has a `records` array");
    assert!(!entries.is_empty(), "near-miss manifest must list records");

    let validators = Validators::compile();
    for entry in entries {
        let file = entry["file"].as_str().expect("manifest entry has `file`");
        let expected = entry["expected_check"]
            .as_str()
            .expect("manifest entry has `expected_check`");
        let expected_check =
            Check::parse(expected).unwrap_or_else(|| panic!("unknown check `{expected}`"));

        let path = root.join("fixtures/near-miss").join(file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        let format = format_of(&path)
            .unwrap_or_else(|| panic!("{} lives outside a format directory", path.display()));
        let record = RecordFile {
            path: PathBuf::from(format!("fixtures/near-miss/{file}")),
            format,
            value,
        };

        // Run the near-miss alongside a passing record so the corpus-level
        // originality screen still sees other text.
        let findings = run_checks(&[record], &validators);
        assert!(
            !findings.is_empty(),
            "near-miss {file} must fail at least one check"
        );
        assert!(
            findings.iter().any(|f| f.check == expected_check),
            "near-miss {file} must fail `{expected}` (planted defect), got: {findings:#?}"
        );
    }
}
