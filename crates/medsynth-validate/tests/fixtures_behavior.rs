//! #7's positive shards pass the smoke gate. All near-miss probes pass
//! schema/cross-field checks; the wrong keys fail the key-rationale lint.
//! Claim-level omissions require #9's semantic gate, not this smoke lint.

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
    let (files, enum_findings) = json_files(&root.join("fixtures").join(format.slug()));
    assert!(
        enum_findings.is_empty(),
        "fixture directory enumeration must not fail: {enum_findings:?}"
    );
    let paths: Vec<PathBuf> = files
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("valid-"))
        })
        .collect();
    let root2 = root.to_path_buf();
    let (records, load_findings) = load_paths(paths, move |p| {
        p.strip_prefix(&root2).unwrap_or(p).to_path_buf()
    });
    assert!(
        load_findings.is_empty(),
        "valid siblings must load cleanly: {load_findings:?}"
    );
    records
}

#[test]
fn gold_set_passes_every_check() {
    let root = workspace_root();
    let paths = medsynth_validate::Format::all()
        .map(|format| root.join(format!("data/gold/{}.jsonl", format.slug())))
        .to_vec();
    let root2 = root.clone();
    let (records, load_findings) = load_paths(paths, move |p| {
        p.strip_prefix(&root2).unwrap_or(p).to_path_buf()
    });
    let registry = load_registry(&root);
    assert!(records.len() >= 24, "at least 24 positive seeds must load");
    let mut findings = load_findings;
    findings.extend(run_checks(&records, &Validators::compile(), &registry));
    assert!(
        findings.is_empty(),
        "gold set must pass every check, got: {findings:#?}"
    );
}

#[test]
fn near_miss_probes_preserve_the_schema_verification_distinction() {
    let root = workspace_root();
    let manifest_path = root.join("data/gold/near-miss/manifest.json");
    let text = std::fs::read_to_string(&manifest_path).expect("near-miss manifest exists");
    let manifest: serde_json::Value =
        serde_json::from_str(&text).expect("near-miss manifest parses");
    let entries = manifest["records"]
        .as_array()
        .expect("manifest has a `records` array");
    assert!(
        entries.len() >= 6,
        "near-miss manifest must list six probes"
    );

    let validators = Validators::compile();
    let registry: HashSet<String> = load_registry(&root);
    for entry in entries {
        let file = entry["file"].as_str().expect("manifest entry has `file`");
        let expected_check = entry["ci_check"].as_str().map(|expected| {
            Check::parse(expected).unwrap_or_else(|| panic!("unknown check `{expected}`"))
        });

        let path = root.join("data/gold/near-miss").join(file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let format = format_of(&path)
            .unwrap_or_else(|| panic!("{} lives outside a format directory", path.display()));
        let display = PathBuf::from(format!("data/gold/near-miss/{file}"));

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
            .filter(|f| f.path.starts_with("data/gold/near-miss"))
            .collect();
        assert!(
            mine.iter().all(|f| f.check != Check::Schema),
            "near-miss {file} must be schema-valid: {mine:#?}"
        );
        match expected_check {
            Some(expected) => assert!(
                mine.len() == 1 && mine[0].check == expected,
                "near-miss {file} must fail only `{expected}`, got: {mine:#?}"
            ),
            None => assert!(
                mine.is_empty(),
                "near-miss {file} needs semantic verification, not a second smoke defect: {mine:#?}"
            ),
        }
    }
}
