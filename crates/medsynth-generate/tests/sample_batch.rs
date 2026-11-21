mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{load_sample_plan, read_json, run_all, workspace_root};
use medsynth_generate::hash::{file_sha256, sha256_str};
use medsynth_generate::validate::{DraftValidator, ExpectedProvenance, ValidationContext};
use medsynth_generate::{Format, SourceRegistry};
use serde_json::Value;

fn collect_anchors(value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(object) => {
            if let (Some(source_id), Some(locator)) = (
                object.get("source_id").and_then(Value::as_str),
                object.get("locator").and_then(Value::as_str),
            ) {
                out.push((source_id.to_owned(), locator.to_owned()));
            }
            for child in object.values() {
                collect_anchors(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_anchors(item, out);
            }
        }
        _ => {}
    }
}

#[test]
fn sample_batch_reproduces_byte_for_byte_with_current_code() {
    let output = run_all(&load_sample_plan());
    let mut regenerated = String::new();
    for record in &output.records {
        regenerated.push_str(&record.raw);
        regenerated.push('\n');
    }
    let committed =
        std::fs::read_to_string(workspace_root().join("drafts/sample-batch/mcq.jsonl")).unwrap();
    assert_eq!(
        regenerated, committed,
        "the committed sample batch is stale; regenerate it"
    );
    assert_eq!(output.manifest.dropped_by_self_validation, 0);
}

#[test]
fn sample_batch_manifest_matches_records_and_current_inputs() {
    let root = workspace_root();
    let dir = root.join("drafts/sample-batch");
    let manifest = read_json(&dir.join("manifest.json"));
    let config = read_json(&dir.join("config.json"));

    assert_eq!(manifest["manifest_version"], 1);
    assert_eq!(manifest["seed"], config["seed"]);
    assert_eq!(
        manifest["config_sha256"].as_str().unwrap(),
        sha256_str(&serde_json::to_string(&manifest["config"]).unwrap())
    );

    for (key, path) in [
        ("taxonomy", "docs/taxonomy.md"),
        ("sources", "docs/sources.json"),
        ("teachers", "docs/teachers.md"),
    ] {
        assert_eq!(
            manifest[key]["sha256"].as_str().unwrap(),
            file_sha256(&root.join(path)).unwrap(),
            "{key} hash drifted; regenerate the sample batch"
        );
    }

    assert_eq!(manifest["teacher_lane"]["status"], "not_requested");
    assert!(manifest["teacher_pin"].is_null());
    assert_eq!(manifest["dropped_by_self_validation"], 0);
    assert_eq!(manifest["dropped_by_generation"], 0);
    assert!(manifest["dropped"].as_array().unwrap().is_empty());
    assert!(manifest["rights_envelopes"].as_array().unwrap().is_empty());

    let record_entries = manifest["records"].as_array().unwrap();
    assert_eq!(
        manifest["totals"]["produced"].as_u64().unwrap() as usize,
        record_entries.len()
    );
    assert!(record_entries.len() >= 8);

    let validator = DraftValidator::load(&root.join("schemas")).unwrap();
    let registry = SourceRegistry::load(&root.join("docs/sources.json")).unwrap();
    let mut produced_by_format: BTreeMap<String, usize> = BTreeMap::new();
    let mut domains: BTreeSet<String> = BTreeSet::new();
    let mut seen_ids = BTreeSet::new();

    for format in Format::all() {
        let path = dir.join(format!("{}.jsonl", format.slug()));
        if !path.exists() {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let mut count = 0usize;
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            count += 1;
            let value: Value = serde_json::from_str(line).unwrap();
            let id = value["id"].as_str().unwrap();
            assert!(seen_ids.insert(id.to_owned()), "duplicate id {id}");
            assert_eq!(value["verification_status"], "unverified");
            assert_eq!(value["schema_version"], 1);
            assert_eq!(value["taxonomy_version"], "0.1");
            assert_eq!(value["contract_version"], "0.1");
            assert_eq!(value["provenance"]["model"], Value::Null);
            assert_eq!(value["provenance"]["prompt_version"], Value::Null);
            assert!(
                value["provenance"]["template_version"]
                    .as_str()
                    .is_some_and(|version| !version.is_empty())
            );

            let entry = record_entries
                .iter()
                .find(|entry| entry["id"] == id)
                .unwrap_or_else(|| panic!("manifest lacks `{id}`"));
            assert_eq!(entry["sha256"], sha256_str(line));
            assert_eq!(
                entry["jsonl_file"],
                path.file_name().unwrap().to_str().unwrap()
            );
            assert_eq!(entry["format"], format.slug());

            let context = ValidationContext {
                expected: ExpectedProvenance::Procedural,
                registry: &registry,
                anchor_allowlist: None,
            };
            validator.validate_raw(format, line, &context).unwrap();

            let mut anchors = Vec::new();
            collect_anchors(&value, &mut anchors);
            assert!(!anchors.is_empty(), "record {id} has no anchors");
            for (source_id, locator) in anchors {
                let receipt = manifest["anchor_receipts"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|receipt| {
                        receipt["record_id"] == id
                            && receipt["source_id"] == source_id
                            && receipt["locator"] == locator
                    })
                    .unwrap_or_else(|| {
                        panic!("missing receipt for {id} / {source_id} / {locator}")
                    });
                assert_eq!(
                    receipt["registry_entry_sha256"].as_str().unwrap(),
                    registry.entry_sha256(&source_id).unwrap()
                );
                assert_eq!(
                    receipt["registry_version"].as_str().unwrap(),
                    registry.registry_version
                );
            }
            domains.insert(value["domain"].as_str().unwrap().to_owned());
        }
        produced_by_format.insert(format.slug().to_owned(), count);
    }

    assert_eq!(domains.len(), 4);
    for (format, count) in &produced_by_format {
        assert!(*count >= 2, "{format} shard has only {count} drafts");
    }

    for cell in manifest["cells"].as_array().unwrap() {
        let produced = cell["produced"].as_u64().unwrap() as usize;
        let matching = record_entries
            .iter()
            .filter(|entry| {
                entry["domain"] == cell["domain"]
                    && entry["format"] == cell["format"]
                    && entry["tier"] == cell["tier"]
            })
            .count();
        assert_eq!(produced, matching, "cell {cell:?}");
        assert_eq!(cell["requested"], 2);
        assert_eq!(cell["dispatched"], 2);
        assert_eq!(cell["dropped"], 0);
        assert_eq!(cell["status"], "produced");
    }
}
