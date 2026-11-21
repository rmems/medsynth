//! Gold seed staging invariants, not dataset admission or a semantic verifier.
//! The separate reviewer run supplies passage-support and blind-answer evidence.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use medsynth_schema::{CaseVignetteRecord, McqRecord, TutoringExampleRecord};
use serde_json::Value;
use sha2::{Digest, Sha256};

const FORMATS: [&str; 3] = ["mcq", "case_vignette", "tutoring_example"];
const DOMAINS: [&str; 4] = ["bbf", "cpf", "psb", "pre_clinical"];
const CHECKLIST: &str = "gold-review-v1";
const DRAFTER: &str = "factory-droid-draft-rm2522-v1";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&read(path))
        .unwrap_or_else(|e| panic!("{}: invalid JSON: {e}", path.display()))
}

fn validators() -> BTreeMap<&'static str, jsonschema::Validator> {
    FORMATS
        .map(|format| {
            let schema = json(&root().join(format!("schemas/{format}.v1.json")));
            (
                format,
                jsonschema::validator_for(&schema).expect("committed schema compiles"),
            )
        })
        .into_iter()
        .collect()
}

fn assert_shape(
    format: &str,
    raw: &str,
    validator: &jsonschema::Validator,
    location: &str,
) -> Value {
    let value: Value =
        serde_json::from_str(raw).unwrap_or_else(|e| panic!("{location}: invalid JSON: {e}"));
    let errors: Vec<_> = validator
        .iter_errors(&value)
        .map(|e| format!("{}: {e}", e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{location}: {errors:?}");
    // Deserialize the original text, not Value: duplicate fields must not
    // collapse before the typed consumer sees them.
    let result = match format {
        "mcq" => serde_json::from_str::<McqRecord>(raw)
            .map(|r| r.validate())
            .map_err(|e| e.to_string()),
        "case_vignette" => serde_json::from_str::<CaseVignetteRecord>(raw)
            .map(|r| r.validate())
            .map_err(|e| e.to_string()),
        "tutoring_example" => serde_json::from_str::<TutoringExampleRecord>(raw)
            .map(|r| r.validate())
            .map_err(|e| e.to_string()),
        _ => panic!("{location}: unknown format {format}"),
    };
    let cross = result.unwrap_or_else(|e| panic!("{location}: deserialization: {e}"));
    assert!(cross.is_ok(), "{location}: cross-field: {cross:?}");
    assert_eq!(value["verification_status"], "unverified", "{location}");
    assert_eq!(value["taxonomy_version"], "0.1", "{location}");
    assert_eq!(value["contract_version"], "0.1", "{location}");
    assert_eq!(
        value["provenance"]["generator"],
        format!("{DRAFTER}; checklist={CHECKLIST}"),
        "{location}"
    );
    for field in ["model", "prompt_version", "template_version"] {
        assert!(value["provenance"][field].is_null(), "{location}: {field}");
    }
    value
}

fn anchors(value: &Value, out: &mut Vec<Value>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if key == "anchors" {
                    out.extend(
                        child
                            .as_array()
                            .expect("anchors must be an array (schema-validated)")
                            .iter()
                            .cloned(),
                    );
                } else {
                    anchors(child, out);
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|v| anchors(v, out)),
        _ => {}
    }
}

/// Registry convention syntax only. Resolving a chapter/section and checking
/// its claim support is deliberately left to the independent review and #9.
fn locator_pattern(convention: &str) -> &'static str {
    match convention {
        "ch<N>:<section-slug> or p.<page>" => {
            r"^(ch[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*|p\.[1-9][0-9]*)(?![\s\S])"
        }
        "ch<N>:<section-slug> or sec<N>.<M>:<section-slug>" => {
            r"^(ch[1-9][0-9]*|sec[1-9][0-9]*\.[1-9][0-9]*):[a-z0-9]+(-[a-z0-9]+)*(?![\s\S])"
        }
        "ch<N>:<section-slug>" => r"^ch[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*(?![\s\S])",
        "vol<N>:ch<N>:<section-slug>" => {
            r"^vol[1-3]:ch[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*(?![\s\S])"
        }
        "sec<N>.<M>:<section-slug> or rec<N>" => {
            r"^(sec[1-9][0-9]*\.[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*|rec[1-9][0-9]*)(?![\s\S])"
        }
        "sec<N>:<title-slug>@YYYY-MM-DD" => {
            r"^sec[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*@[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])(?![\s\S])"
        }
        "rec<N> or sec<N>:<title-slug>" => {
            r"^(rec[1-9][0-9]*|sec[1-9][0-9]*:[a-z0-9]+(-[a-z0-9]+)*)(?![\s\S])"
        }
        _ => panic!("unhandled registry locator convention: {convention}"),
    }
}

fn assert_registry_anchors(value: &Value, registry: &Value) {
    let mut cited = Vec::new();
    anchors(value, &mut cited);
    assert!(!cited.is_empty(), "{}: no anchors", value["id"]);
    for anchor in cited {
        let source = registry["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["source_id"] == anchor["source_id"])
            .unwrap_or_else(|| panic!("{}: unlisted source {anchor}", value["id"]));
        assert_eq!(source["status"], "current", "{}: {anchor}", value["id"]);
        let pattern = locator_pattern(source["locator_convention"]["format"].as_str().unwrap());
        let validator =
            jsonschema::validator_for(&serde_json::json!({"type": "string", "pattern": pattern}))
                .unwrap();
        assert!(
            validator.is_valid(&anchor["locator"]),
            "{}: locator violates registry convention: {anchor}",
            value["id"]
        );
    }
}

fn gold_lines() -> Vec<(String, String)> {
    let mut lines = Vec::new();
    for format in FORMATS {
        let path = root().join(format!("data/gold/{format}.jsonl"));
        let text = read(&path);
        assert!(!text.is_empty(), "{}: empty shard", path.display());
        for (i, line) in text.lines().enumerate() {
            assert!(
                !line.trim().is_empty(),
                "{}:{}: blank JSONL line",
                path.display(),
                i + 1
            );
            lines.push((format.to_owned(), line.to_owned()));
        }
    }
    lines
}

fn near_entries() -> Value {
    json(&root().join("data/gold/near-miss/manifest.json"))
}

#[test]
fn gold_schema_registry_and_coverage() {
    let validators = validators();
    let registry = json(&root().join("docs/sources.json"));
    let mut ids = BTreeSet::new();
    let mut cells: BTreeMap<(String, String), Vec<Value>> = BTreeMap::new();
    let mut format_tiers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (format, raw) in gold_lines() {
        let value = assert_shape(&format, &raw, &validators[format.as_str()], &format);
        assert_registry_anchors(&value, &registry);
        let id = value["id"].as_str().unwrap();
        assert!(id.starts_with("gold-"), "{id}: not a gold id");
        assert!(ids.insert(id.to_owned()), "{id}: duplicate id");
        format_tiers
            .entry(format.clone())
            .or_default()
            .insert(value["tier"].as_str().unwrap().to_owned());
        cells
            .entry((format, value["domain"].as_str().unwrap().to_owned()))
            .or_default()
            .push(value);
    }
    assert!(ids.len() >= 24, "at least 24 gold records required");
    for format in FORMATS {
        let tiers = format_tiers
            .get(format)
            .unwrap_or_else(|| panic!("{format}: no records for format"));
        assert_eq!(tiers.len(), 3, "{format}: all tiers required");
        for domain in DOMAINS {
            let cell = cells
                .get(&(format.to_owned(), domain.to_owned()))
                .unwrap_or_else(|| panic!("{format}/{domain}: cell missing"));
            assert!(cell.len() >= 2, "{format}/{domain}: at least two records");
            for field in ["subdomain", "tier"] {
                let values: BTreeSet<_> = cell.iter().map(|r| r[field].as_str().unwrap()).collect();
                assert!(
                    values.len() >= 2,
                    "{format}/{domain}: spread across {field} required"
                );
            }
        }
    }
}

#[test]
fn near_misses_are_schema_valid_and_manifested() {
    let validators = validators();
    let manifest = near_entries();
    assert_eq!(manifest["checklist_id"], CHECKLIST);
    let registry = json(&root().join("docs/sources.json"));
    let entries = manifest["records"].as_array().unwrap();
    assert!(entries.len() >= 6);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut files = BTreeSet::new();
    let mut ids = BTreeSet::new();
    for entry in entries {
        let file = entry["file"].as_str().unwrap();
        assert!(
            Path::new(file)
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_))),
            "near-miss paths must be relative and contained: {file}"
        );
        assert!(
            files.insert(file.to_owned()),
            "{file}: duplicate manifest file"
        );
        let format = entry["format"].as_str().unwrap();
        assert!(file.starts_with(&format!("{format}/")));
        let path = root().join("data/gold/near-miss").join(file);
        let value = assert_shape(format, &read(&path), &validators[format], file);
        assert_registry_anchors(&value, &registry);
        assert_eq!(entry["id"], value["id"], "{file}");
        let id = value["id"].as_str().unwrap();
        assert!(id.starts_with("near-miss-"));
        assert!(ids.insert(id.to_owned()));
        let defect = entry["defect"].as_str().unwrap();
        *counts.entry(defect.to_owned()).or_default() += 1;
        match defect {
            "wrong_key" => {
                assert_eq!(entry["verification_stage"], 2);
                assert_ne!(value["key"], entry["expected_answer"]);
            }
            "missing_anchor" => {
                assert_eq!(entry["verification_stage"], 3);
                assert_eq!(
                    value
                        .pointer(entry["defect_path"].as_str().unwrap())
                        .unwrap(),
                    &serde_json::json!([])
                );
            }
            "unanchored_claim" => {
                assert_eq!(entry["verification_stage"], 3);
                assert!(
                    !value["tutor_response"]["anchors"]
                        .as_array()
                        .unwrap()
                        .is_empty(),
                    "{file}: partial support must not become an empty-anchor defect"
                );
                assert!(
                    value
                        .pointer(entry["defect_path"].as_str().unwrap())
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_lowercase()
                        .contains(&entry["unsupported_claim"].as_str().unwrap().to_lowercase())
                );
            }
            _ => panic!("{file}: unknown defect {defect}"),
        }
    }
    for defect in ["wrong_key", "missing_anchor", "unanchored_claim"] {
        assert!(
            counts[defect] >= 2,
            "{defect}: at least two probes required"
        );
    }
    let mut actual = BTreeSet::new();
    let near_root = root().join("data/gold/near-miss");
    collect_probes(&near_root, &near_root, &mut actual);
    assert_eq!(files, actual, "every near-miss must be in the manifest");
}

/// Recursively collect probe files under the near-miss root, rejecting
/// anything that is not manifest.json, README.md, or a .json probe inside
/// a format directory — a probe the sweep cannot see must not be able to
/// hide from the completeness check above.
fn collect_probes(root_dir: &Path, dir: &Path, actual: &mut BTreeSet<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        if path.is_dir() {
            assert!(
                FORMATS.contains(&name.as_str()),
                "unexpected near-miss directory: {}",
                path.display()
            );
            collect_probes(root_dir, &path, actual);
        } else if dir == root_dir {
            assert!(
                name == "manifest.json" || name == "README.md",
                "unexpected file at near-miss root: {name}"
            );
        } else {
            assert!(
                path.extension().is_some_and(|e| e == "json"),
                "unexpected file in near-miss dir: {}",
                path.display()
            );
            let rel = path
                .strip_prefix(root_dir)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            actual.insert(rel);
        }
    }
}

fn assert_review(entry: &Value, reviewers: &[&str], raw: &[u8]) {
    assert!(
        reviewers.contains(&entry["reviewer_run_id"].as_str().unwrap()),
        "{}: entry names a reviewer run the manifest does not",
        entry["record_id"]
    );
    let digest = format!("{:x}", Sha256::digest(raw));
    assert_eq!(
        entry["record_sha256"], digest,
        "{}: record changed since independent review",
        entry["record_id"]
    );
}

#[test]
fn independent_review_covers_the_exact_records() {
    let manifest = json(&root().join("data/gold/review-manifest.json"));
    assert_eq!(manifest["checklist_id"], CHECKLIST);
    assert_eq!(manifest["drafting_run_id"], DRAFTER);
    assert_eq!(manifest["review_status"], "passed");
    let reviewer = manifest["reviewer_run_id"].as_str().unwrap();
    // " + " separates the reviewing runs when more than one produced
    // evidence (for example a re-review after changes_requested); each
    // entry then names the run that reviewed its exact pinned bytes.
    let reviewers: Vec<&str> = reviewer.split(" + ").collect();
    assert!(
        reviewers
            .iter()
            .all(|r| !r.trim().is_empty() && *r != DRAFTER)
    );
    let mut reviewed = BTreeMap::new();
    for entry in manifest["records"].as_array().unwrap() {
        let id = entry["record_id"].as_str().unwrap();
        assert!(
            reviewed.insert(id.to_owned(), entry).is_none(),
            "{id}: duplicate review"
        );
    }
    let lines = gold_lines();
    assert_eq!(reviewed.len(), lines.len(), "one review per gold record");
    for (format, raw) in lines {
        let value: Value = serde_json::from_str(&raw).unwrap();
        let id = value["id"].as_str().unwrap();
        let entry = reviewed
            .get(id)
            .unwrap_or_else(|| panic!("{id}: missing review"));
        assert_review(entry, &reviewers, raw.as_bytes());
        for check in ["anchor_check", "rationale_check", "tier_check"] {
            assert_eq!(entry[check]["outcome"], "pass", "{id}: {check}");
        }
        let key_outcome = if format == "tutoring_example" {
            "not_applicable"
        } else {
            "pass"
        };
        assert_eq!(entry["key_check"]["outcome"], key_outcome, "{id}");
        if format == "mcq" {
            assert_eq!(entry["key_check"]["blind_answer"], value["key"], "{id}");
            assert_eq!(entry["distractor_check"]["outcome"], "pass", "{id}");
        } else {
            assert!(
                matches!(
                    entry["distractor_check"]["outcome"].as_str(),
                    Some("pass" | "not_applicable")
                ),
                "{id}: distractor review"
            );
        }
        for check in [
            "anchor_check",
            "key_check",
            "rationale_check",
            "distractor_check",
            "tier_check",
        ] {
            assert!(
                entry[check]["evidence"]
                    .as_str()
                    .is_some_and(|s| !s.trim().is_empty()),
                "{id}: missing evidence for {check}"
            );
        }
    }
    let mut reviewed_near = BTreeMap::new();
    for entry in manifest["near_misses"].as_array().unwrap() {
        let id = entry["record_id"].as_str().unwrap();
        assert!(reviewed_near.insert(id.to_owned(), entry).is_none());
    }
    let near = near_entries();
    let entries = near["records"].as_array().unwrap();
    assert_eq!(reviewed_near.len(), entries.len());
    for probe in entries {
        let id = probe["id"].as_str().unwrap();
        let entry = reviewed_near
            .get(id)
            .unwrap_or_else(|| panic!("{id}: missing review"));
        let path = root()
            .join("data/gold/near-miss")
            .join(probe["file"].as_str().unwrap());
        assert_review(entry, &reviewers, &fs::read(path).unwrap());
        assert_eq!(entry["planted_defect_check"]["outcome"], "pass", "{id}");
        assert!(
            entry["planted_defect_check"]["evidence"]
                .as_str()
                .is_some_and(|s| !s.trim().is_empty()),
            "{id}: no defect confirmation"
        );
    }
}
