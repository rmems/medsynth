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
//!   JSON entry (and vice versa), with matching status, coverage, citation,
//!   kind, added-by, and access date;
//! * every taxonomy §3.2 subdomain (read from `docs/taxonomy.md`, so
//!   taxonomy amendments are picked up automatically) has at least one
//!   `current` approved source;
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
    kind: String,
    status: String,
    coverage: Vec<String>,
    #[serde(default)]
    citation: Option<String>,
    #[serde(default)]
    access_date: Option<String>,
    added_by: String,
}

/// The fields `docs/sources.md` must agree with `sources.json` on.
#[derive(Debug, PartialEq)]
struct MdEntry {
    status: String,
    coverage: Vec<String>,
    citation: Option<String>,
    kind: String,
    added_by: String,
    access_date: Option<String>,
}

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

fn first_token(value: &str) -> String {
    value
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
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
        let status = bullet_value(section, "- **Status:**").map(first_token);
        let kind = bullet_value(section, "- **Kind:**").map(first_token);
        let added_by = bullet_value(section, "- **Added by:**").map(str::to_string);
        let access_date = bullet_value(section, "- **Access date:**").map(first_token);
        let coverage = bullet_value(section, "- **Coverage:**")
            .map(backtick_slugs)
            .unwrap_or_default();
        let citation = bullet_value(section, "- **Citation:**").map(str::to_string);
        entries.insert(
            id.clone(),
            MdEntry {
                status: status.unwrap_or_default(),
                coverage,
                citation,
                kind: kind.unwrap_or_default(),
                added_by: added_by.unwrap_or_default(),
                access_date,
            },
        );
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
    let mut files: Vec<PathBuf> = fs::read_dir(&fixtures)
        .expect("cannot read fixtures/")
        .map(|d| d.expect("dir entry").path())
        .filter(|p| p.is_dir())
        .flat_map(|dir| {
            fs::read_dir(&dir)
                .expect("cannot read fixture dir")
                .map(|f| f.expect("dir entry").path())
                .collect::<Vec<_>>()
        })
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("valid-") && n.ends_with(".json"))
        })
        .collect();
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

/// The subdomain slugs declared by the `Subdomain` enum, read from the
/// generated JSON schema so they track the enum itself rather than a
/// hand-maintained list.
fn schema_subdomains() -> BTreeSet<String> {
    let path = workspace_root().join("schemas").join("mcq.v1.json");
    let schema: Value = serde_json::from_str(&read_text(&path)).expect("parse mcq.v1.json schema");
    schema["$defs"]["Subdomain"]["oneOf"]
        .as_array()
        .expect("Subdomain schema has oneOf groups")
        .iter()
        .flat_map(|group| {
            // Groups render either as a shared `enum` or per-variant `const`.
            group["enum"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| {
                    group["const"]
                        .as_str()
                        .map(|c| vec![c.to_string()])
                        .unwrap_or_default()
                })
        })
        .collect()
}

/// The taxonomy §3.2 subdomain slugs, read from the `subdomain:` YAML block
/// in `docs/taxonomy.md` so amendments are picked up without touching this
/// test.
fn taxonomy_subdomains() -> BTreeSet<String> {
    let text = read_text(&workspace_root().join("docs").join("taxonomy.md"));
    let section = text
        .split("### 3.2")
        .nth(1)
        .and_then(|rest| rest.split("### 3.3").next())
        .expect("taxonomy.md has no §3.2 subdomain section");
    section
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("- "))
        .filter_map(|rest| rest.split('#').next())
        .filter_map(|rest| rest.split_whitespace().next())
        .map(str::to_string)
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

    for (id, md_entry) in &md {
        let entry = json[id.as_str()];
        assert_eq!(
            entry.status, md_entry.status,
            "status mismatch for `{id}`: md `{}` vs json `{}`",
            md_entry.status, entry.status
        );
        assert_eq!(
            entry.kind, md_entry.kind,
            "kind mismatch for `{id}`: md `{}` vs json `{}`",
            md_entry.kind, entry.kind
        );
        assert_eq!(
            entry.added_by, md_entry.added_by,
            "added_by mismatch for `{id}`: md `{}` vs json `{}`",
            md_entry.added_by, entry.added_by
        );
        assert_eq!(
            entry.access_date.as_deref(),
            md_entry.access_date.as_deref(),
            "access_date mismatch for `{id}`: md {:?} vs json {:?}",
            md_entry.access_date,
            entry.access_date
        );
        let json_coverage: BTreeSet<&str> = entry.coverage.iter().map(String::as_str).collect();
        let md_coverage: BTreeSet<&str> = md_entry.coverage.iter().map(String::as_str).collect();
        assert_eq!(
            json_coverage, md_coverage,
            "coverage mismatch for `{id}`: md {:?} vs json {:?}",
            md_entry.coverage, entry.coverage
        );
        assert_eq!(
            entry.citation.as_deref(),
            md_entry.citation.as_deref(),
            "citation mismatch for `{id}`: md {:?} vs json {:?}",
            md_entry.citation,
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
    let doc_slugs = taxonomy_subdomains();
    let enum_slugs = schema_subdomains();
    assert_eq!(
        doc_slugs, enum_slugs,
        "docs/taxonomy.md §3.2 subdomains and the generated Subdomain schema disagree"
    );
    for slug in &enum_slugs {
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
