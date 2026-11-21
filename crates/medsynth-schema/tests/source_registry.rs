//! Checks for the anchor source registry (#5): `docs/sources.json` is the
//! machine-readable copy of `docs/sources.md`, and both must stay
//! entry-for-entry identical — a disagreement is a defect in the PR that
//! introduced it.
//!
//! This test proves the wiring #4/#9 need:
//!
//! * every registry `source_id` matches `SOURCE_ID_PATTERN`;
//! * every `coverage` slug is a taxonomy §3.2 subdomain;
//! * every `### \`id\`` entry heading in `docs/sources.md` corresponds to a
//!   JSON entry (and vice versa), with matching status and coverage;
//! * every `source_id` cited by a `valid-*.json` fixture exists in the
//!   registry. (`invalid-*.json` fixtures are exempt — they carry
//!   deliberately broken fields.)

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use medsynth_schema::{SOURCE_ID_PATTERN, SourceId, Subdomain};
use serde::Deserialize;
use serde_json::Value;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .to_path_buf()
}

#[derive(Debug, Deserialize)]
struct Registry {
    sources: Vec<RegistryEntry>,
}

#[derive(Debug, Deserialize)]
struct RegistryEntry {
    source_id: String,
    status: String,
    coverage: Vec<String>,
}

fn registry() -> Registry {
    let path = workspace_root().join("docs").join("sources.json");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Entries in `docs/sources.md` under `## Registry entries`, keyed by the
/// `### \`source-id\`` heading. Returns (source_id, status, coverage).
fn markdown_entries() -> BTreeMap<String, (String, Vec<String>)> {
    let path = workspace_root().join("docs").join("sources.md");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut entries = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("### `") {
            let id = rest
                .strip_suffix('`')
                .expect("entry heading must end with a backtick")
                .to_string();
            current = Some(id.clone());
            entries.insert(id, (String::new(), Vec::new()));
        } else if let Some(id) = current.clone() {
            if let Some(value) = line.strip_prefix("- **Status:**") {
                let status = value
                    .split_whitespace()
                    .next()
                    .expect("status line is empty")
                    .to_string();
                entries.get_mut(&id).unwrap().0 = status;
            } else if let Some(value) = line.strip_prefix("- **Coverage:**") {
                let slugs: Vec<String> = value
                    .split('`')
                    .skip(1)
                    .step_by(2)
                    .map(str::to_string)
                    .collect();
                assert!(
                    !slugs.is_empty(),
                    "coverage line for {id} has no backticked slugs"
                );
                entries.get_mut(&id).unwrap().1 = slugs;
            }
        }
    }
    entries
}

fn collect_source_ids(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                if key == "source_id"
                    && let Some(s) = v.as_str()
                {
                    out.insert(s.to_string());
                }
                collect_source_ids(v, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_source_ids(item, out);
            }
        }
        _ => {}
    }
}

fn fixture_source_ids() -> BTreeMap<String, BTreeSet<String>> {
    let fixtures = workspace_root().join("fixtures");
    let mut cited: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for dir in fs::read_dir(&fixtures).expect("cannot read fixtures/") {
        let dir = dir.expect("dir entry").path();
        if !dir.is_dir() {
            continue;
        }
        for file in fs::read_dir(&dir).expect("cannot read fixture dir") {
            let path = file.expect("dir entry").path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if !(name.starts_with("valid-") && name.ends_with(".json")) {
                continue;
            }
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let instance: Value = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
            let mut ids = BTreeSet::new();
            collect_source_ids(&instance, &mut ids);
            if !ids.is_empty() {
                cited.insert(name, ids);
            }
        }
    }
    cited
}

#[test]
fn registry_ids_match_source_id_pattern() {
    let reg = registry();
    assert!(!reg.sources.is_empty(), "registry has no sources");
    let mut seen = BTreeSet::new();
    for entry in &reg.sources {
        assert!(
            seen.insert(entry.source_id.clone()),
            "duplicate source_id `{}` in sources.json",
            entry.source_id
        );
        assert!(
            SourceId::is_valid(&entry.source_id),
            "source_id `{}` does not match {SOURCE_ID_PATTERN}",
            entry.source_id
        );
        assert!(
            matches!(entry.status.as_str(), "current" | "superseded"),
            "source_id `{}` has invalid status `{}`",
            entry.source_id,
            entry.status
        );
    }
}

#[test]
fn registry_coverage_uses_taxonomy_slugs() {
    for entry in &registry().sources {
        assert!(
            !entry.coverage.is_empty(),
            "source_id `{}` claims no coverage",
            entry.source_id
        );
        for slug in &entry.coverage {
            serde_json::from_value::<Subdomain>(Value::String(slug.clone())).unwrap_or_else(|e| {
                panic!(
                    "source_id `{}` coverage slug `{slug}` is not a taxonomy §3.2 subdomain: {e}",
                    entry.source_id
                )
            });
        }
    }
}

#[test]
fn markdown_and_json_are_entry_for_entry_identical() {
    let reg = registry();
    let json_ids: BTreeMap<&str, (&RegistryEntry,)> = reg
        .sources
        .iter()
        .map(|e| (e.source_id.as_str(), (e,)))
        .collect();
    let md = markdown_entries();

    let json_set: BTreeSet<&str> = json_ids.keys().copied().collect();
    let md_set: BTreeSet<&str> = md.keys().map(String::as_str).collect();
    assert_eq!(
        json_set, md_set,
        "sources.md entry headings and sources.json source_ids disagree"
    );

    for (id, (status, coverage)) in &md {
        let entry = &json_ids[id.as_str()].0;
        assert_eq!(
            entry.status, *status,
            "status mismatch for `{id}`: md `{status}` vs json `{}`",
            entry.status
        );
        let json_coverage: BTreeSet<&str> = entry.coverage.iter().map(String::as_str).collect();
        let md_coverage: BTreeSet<&str> = coverage.iter().map(String::as_str).collect();
        assert_eq!(
            json_coverage, md_coverage,
            "coverage mismatch for `{id}`: md {coverage:?} vs json {:?}",
            entry.coverage
        );
    }
}

#[test]
fn every_subdomain_has_an_approved_source() {
    let mut covered = BTreeSet::new();
    for entry in &registry().sources {
        covered.extend(entry.coverage.iter().cloned());
    }
    // The 25 taxonomy §3.2 subdomains; mirrors docs/taxonomy.md.
    const ALL_SUBDOMAINS: [&str; 25] = [
        "bio_physical_foundations",
        "macromolecules",
        "cell_function",
        "cell_processes",
        "system_interactions",
        "animal_structure",
        "translation",
        "properties_of_matter",
        "transmission",
        "mechanics",
        "development",
        "socioeconomic",
        "culture_identity",
        "psychology",
        "environment",
        "cardiovascular",
        "respiratory",
        "renal",
        "gastrointestinal",
        "endocrine",
        "neurology",
        "musculoskeletal",
        "hematology_oncology",
        "infectious_disease",
        "pharmacology_principles",
    ];
    for slug in ALL_SUBDOMAINS {
        assert!(
            covered.contains(slug),
            "taxonomy subdomain `{slug}` has no approved source in docs/sources.json"
        );
    }
}

#[test]
fn fixture_source_ids_exist_in_registry() {
    let reg = registry();
    let registry_ids: BTreeSet<&str> = reg.sources.iter().map(|e| e.source_id.as_str()).collect();
    let cited = fixture_source_ids();
    assert!(
        !cited.is_empty(),
        "no fixture source ids found — did fixtures move?"
    );
    for (file, ids) in &cited {
        for id in ids {
            assert!(
                registry_ids.contains(id.as_str()),
                "{file} cites source_id `{id}` which is not in docs/sources.json"
            );
        }
    }
}
