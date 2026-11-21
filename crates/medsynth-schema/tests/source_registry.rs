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
//!   JSON entry (and vice versa), with matching status, coverage, and
//!   citation;
//! * every taxonomy §3.2 subdomain has at least one `current` approved
//!   source;
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
    #[serde(default)]
    citation: Option<String>,
}

/// What the markdown mirror must agree with: status, coverage slugs, and
/// the optional citation string.
type MdEntry = (String, Vec<String>, Option<String>);

fn read_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn registry() -> Registry {
    let path = workspace_root().join("docs").join("sources.json");
    let text = read_text(&path);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn bullet_value<'a>(lines: &[&'a str], prefix: &str) -> Option<&'a str> {
    lines
        .iter()
        .find_map(|line| line.strip_prefix(prefix))
        .map(str::trim)
}

fn backtick_slugs(value: &str) -> Vec<String> {
    value
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

fn entry_heading(line: &str) -> Option<String> {
    line.strip_prefix("### `")?
        .strip_suffix('`')
        .map(str::to_string)
}

/// Entries in `docs/sources.md` keyed by their `### \`source-id\`` heading.
fn markdown_entries() -> BTreeMap<String, MdEntry> {
    let path = workspace_root().join("docs").join("sources.md");
    let text = read_text(&path);
    let lines: Vec<&str> = text.lines().collect();
    let headings: Vec<(usize, String)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| entry_heading(line).map(|id| (i, id)))
        .collect();
    let mut entries = BTreeMap::new();
    for (idx, (start, id)) in headings.iter().enumerate() {
        let end = headings
            .get(idx + 1)
            .map(|(i, _)| *i)
            .unwrap_or(lines.len());
        let section = &lines[*start..end];
        let status = bullet_value(section, "- **Status:**")
            .and_then(|v| v.split_whitespace().next())
            .unwrap_or_default()
            .to_string();
        let coverage = bullet_value(section, "- **Coverage:**")
            .map(backtick_slugs)
            .unwrap_or_default();
        let citation = bullet_value(section, "- **Citation:**").map(str::to_string);
        entries.insert(id.clone(), (status, coverage, citation));
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

fn valid_fixture_files() -> Vec<PathBuf> {
    let fixtures = workspace_root().join("fixtures");
    let mut files = Vec::new();
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
                .unwrap_or_default();
            if name.starts_with("valid-") && name.ends_with(".json") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn cited_source_ids(path: &Path) -> BTreeSet<String> {
    let text = read_text(path);
    let instance: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
    let mut ids = BTreeSet::new();
    collect_source_ids(&instance, &mut ids);
    ids
}

/// `source_id`s cited by each `valid-*.json` fixture, keyed by fixture path
/// so same-named fixtures in different format dirs cannot mask each other.
fn fixture_source_ids() -> BTreeMap<String, BTreeSet<String>> {
    valid_fixture_files()
        .into_iter()
        .map(|path| (path.display().to_string(), cited_source_ids(&path)))
        .filter(|(_, ids)| !ids.is_empty())
        .collect()
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
    let json: BTreeMap<&str, &RegistryEntry> = reg
        .sources
        .iter()
        .map(|e| (e.source_id.as_str(), e))
        .collect();
    let md = markdown_entries();

    let json_set: BTreeSet<&str> = json.keys().copied().collect();
    let md_set: BTreeSet<&str> = md.keys().map(String::as_str).collect();
    assert_eq!(
        json_set, md_set,
        "sources.md entry headings and sources.json source_ids disagree"
    );

    for (id, (status, coverage, citation)) in &md {
        let entry = json[id.as_str()];
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
        assert_eq!(
            entry.citation.as_deref(),
            citation.as_deref(),
            "citation mismatch for `{id}`: md {citation:?} vs json {:?}",
            entry.citation
        );
    }
}

#[test]
fn every_subdomain_has_a_current_source() {
    let mut covered = BTreeSet::new();
    for entry in &registry().sources {
        if entry.status == "current" {
            covered.extend(entry.coverage.iter().cloned());
        }
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
            "taxonomy subdomain `{slug}` has no current approved source in docs/sources.json"
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
