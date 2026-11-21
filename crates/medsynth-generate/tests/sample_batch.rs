mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{load_sample_plan, read_json, run_all, workspace_root};
use medsynth_generate::batch::anchors::collect_anchors;
use medsynth_generate::hash::{sha256_str, text_file_sha256};
use medsynth_generate::validate::{DraftValidator, ExpectedProvenance, ValidationContext};
use medsynth_generate::{Format, SourceRegistry};
use serde_json::Value;

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

struct ShardCheck<'a> {
    manifest: &'a Value,
    validator: &'a DraftValidator,
    registry: &'a SourceRegistry,
    record_entries: &'a [Value],
    seen_ids: &'a mut BTreeSet<String>,
    domains: &'a mut BTreeSet<String>,
}

/// One committed draft line: record invariants, the manifest entry it maps
/// to, schema validation, and a receipt for every anchor it cites.
fn check_shard_record(check: &mut ShardCheck<'_>, format: Format, line: &str, file_name: &str) {
    let value: Value = serde_json::from_str(line).unwrap();
    let id = value["id"].as_str().unwrap();
    assert!(check.seen_ids.insert(id.to_owned()), "duplicate id {id}");
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

    let entry = check
        .record_entries
        .iter()
        .find(|entry| entry["id"] == id)
        .unwrap_or_else(|| panic!("manifest lacks `{id}`"));
    assert_eq!(entry["sha256"], sha256_str(line));
    assert_eq!(entry["jsonl_file"], file_name);
    assert_eq!(entry["format"], format.slug());

    let context = ValidationContext {
        expected: ExpectedProvenance::Procedural,
        registry: check.registry,
        anchor_allowlist: None,
    };
    check
        .validator
        .validate_raw(format, line, &context)
        .unwrap();

    let mut anchors = Vec::new();
    collect_anchors(&value, &mut anchors);
    assert!(!anchors.is_empty(), "record {id} has no anchors");
    for (source_id, locator) in anchors {
        let receipt = check.manifest["anchor_receipts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|receipt| {
                receipt["record_id"] == id
                    && receipt["source_id"] == source_id
                    && receipt["locator"] == locator
            })
            .unwrap_or_else(|| panic!("missing receipt for {id} / {source_id} / {locator}"));
        assert_eq!(
            receipt["registry_entry_sha256"].as_str().unwrap(),
            check.registry.entry_sha256(&source_id).unwrap()
        );
        assert_eq!(
            receipt["registry_version"].as_str().unwrap(),
            check.registry.registry_version
        );
    }
    check
        .domains
        .insert(value["domain"].as_str().unwrap().to_owned());
}

/// Manifest-level invariants: version, config hash, input file hashes, lane
/// totals, and every cell reporting produced == matching manifest entries.
fn check_manifest(manifest: &Value, config: &Value, root: &std::path::Path) {
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
            text_file_sha256(&root.join(path)).unwrap(),
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
        let requested = cell["requested"].as_u64().unwrap();
        assert!(requested >= 1);
        assert_eq!(cell["dispatched"], requested);
        assert_eq!(cell["dropped"], 0);
        assert_eq!(cell["status"], "produced");
    }
}

#[test]
fn sample_batch_manifest_matches_records_and_current_inputs() {
    let root = workspace_root();
    let dir = root.join("drafts/sample-batch");
    let manifest = read_json(&dir.join("manifest.json"));
    let config = read_json(&dir.join("config.json"));
    check_manifest(&manifest, &config, &root);

    let validator = DraftValidator::load(&root.join("schemas")).unwrap();
    let registry = SourceRegistry::load(&root.join("docs/sources.json")).unwrap();
    let mut seen_ids = BTreeSet::new();
    let mut domains: BTreeSet<String> = BTreeSet::new();
    let mut produced_by_format: BTreeMap<String, usize> = BTreeMap::new();
    let mut check = ShardCheck {
        manifest: &manifest,
        validator: &validator,
        registry: &registry,
        record_entries: manifest["records"].as_array().unwrap(),
        seen_ids: &mut seen_ids,
        domains: &mut domains,
    };

    for format in Format::all() {
        let path = dir.join(format!("{}.jsonl", format.slug()));
        if !path.exists() {
            continue;
        }
        let file_name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        let mut count = 0usize;
        for line in text.lines().filter(|line| !line.trim().is_empty()) {
            count += 1;
            check_shard_record(&mut check, format, line, &file_name);
        }
        produced_by_format.insert(format.slug().to_owned(), count);
    }

    assert_eq!(check.domains.len(), 4);
    for (format, count) in &produced_by_format {
        assert!(*count >= 2, "{format} shard has only {count} drafts");
    }
}
