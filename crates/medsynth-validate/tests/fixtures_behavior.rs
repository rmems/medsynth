//! Fixture-behavior tests keyed to #7's artifacts: every gold record under
//! `data/gold/` passes every check, and every near-miss record named in
//! `fixtures/near-miss/manifest.json` fails with the check matching its
//! planted defect. Until #7 lands, both tests skip with a message rather
//! than fabricating records.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use medsynth_validate::{
    Check, RecordFile, Validators, format_of, json_files, load_paths, load_registry, run_checks,
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .to_path_buf()
}

/// The `valid-*` fixture records of one format, loaded like the gate loads
/// them (`.json` and `.jsonl` alike).
fn valid_siblings(root: &Path, format: medsynth_validate::Format) -> Vec<RecordFile> {
    let paths: Vec<PathBuf> = json_files(&root.join("fixtures").join(format.slug()))
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("valid-"))
        })
        .collect();
    let root2 = root.to_path_buf();
    load_paths(paths, move |p| {
        p.strip_prefix(&root2).unwrap_or(p).to_path_buf()
    })
    .0
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
    let registry = load_registry(&root);
    let mut findings = load_findings;
    findings.extend(run_checks(&records, &Validators::compile(), &registry));
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
    let registry: HashSet<String> = load_registry(&root);
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
        let format = format_of(&path)
            .unwrap_or_else(|| panic!("{} lives outside a format directory", path.display()));
        let display = PathBuf::from(format!("fixtures/near-miss/{file}"));

        // Run the near-miss alongside the other valid records of its
        // format so the corpus-level checks (originality, duplicate ids)
        // see a realistic slice.
        let mut slice = valid_siblings(&root, format);
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
        slice.push(RecordFile {
            path: display,
            line: None,
            format,
            value,
            raw: text,
        });

        let findings = run_checks(&slice, &validators, &registry);
        let mine: Vec<_> = findings
            .iter()
            .filter(|f| f.path.starts_with("fixtures/near-miss"))
            .collect();
        assert!(
            !mine.is_empty(),
            "near-miss {file} must fail at least one check"
        );
        assert!(
            mine.iter().any(|f| f.check == expected_check),
            "near-miss {file} must fail `{expected}` (planted defect), got: {mine:#?}"
        );
    }
}
