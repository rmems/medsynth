//! The medsynth data-validation gate.
//!
//! One binary runs every check over the record corpus — every record under
//! `data/` (including `data/gold/`; `.json` files and `.jsonl` shards, one
//! record per line per `data/README.md`) and every `valid-*.json` fixture
//! under `fixtures/` — and reports every failure it finds before exiting
//! nonzero. The checks:
//!
//! * **`schema`** — JSON Schema validation against the schemas generated
//!   from `medsynth-schema`'s record types (the single source of truth),
//!   plus deserialization of the original record text into the typed
//!   record (which also rejects duplicate JSON fields) and its
//!   cross-field `validate()`. Duplicate record ids across the corpus are
//!   reported here too.
//! * **`key-rationale`** — rationale present and non-empty and referencing
//!   the keyed answer; answer/response fields non-empty on the formats
//!   that carry them; anchor fields present and non-empty where the format
//!   carries them (`AnchoredClaim` fields).
//! * **`anchor-presence`** — no record whose `verification_status` is above
//!   `unverified` may lack registry anchors entirely: at least one anchor
//!   must cite a `source_id` listed in `docs/sources.json` (contract §4).
//!   The status ordering is the declaration order of `medsynth-schema`'s
//!   [`VerificationStatus`] (`unverified` < `key-checked` <
//!   `anchor-checked` < `admitted`); this gate never advances a status.
//! * **`originality`** — the smoke originality screen documented in
//!   [`originality`]: a cheap deterministic word n-gram overlap screen
//!   over record text fields. The full audit is #11 and out of scope here.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use medsynth_schema::{
    CaseVignetteRecord, McqRecord, TutoringExampleRecord, ValidationError, VerificationStatus,
    schema_files,
};

/// The check a [`Finding`] names. The near-miss manifest's `expected_check`
/// values are these names in kebab-case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Check {
    /// JSON Schema, deserialization, cross-field `validate()`, or a
    /// duplicate record id.
    Schema,
    /// Rationale/answer missing, empty, or not referencing the keyed answer.
    KeyRationale,
    /// A record above `unverified` lacks registry anchors entirely.
    AnchorPresence,
    /// Record text near-duplicates another record in the corpus.
    Originality,
}

impl Check {
    /// The stable name used in report output and near-miss manifests.
    pub fn name(self) -> &'static str {
        match self {
            Self::Schema => "schema",
            Self::KeyRationale => "key-rationale",
            Self::AnchorPresence => "anchor-presence",
            Self::Originality => "originality",
        }
    }

    /// Parse a [`Check::name`].
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "schema" => Some(Self::Schema),
            "key-rationale" => Some(Self::KeyRationale),
            "anchor-presence" => Some(Self::AnchorPresence),
            "originality" => Some(Self::Originality),
            _ => None,
        }
    }
}

impl std::fmt::Display for Check {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// One record format, identified by the taxonomy §3.3 directory slug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Mcq,
    CaseVignette,
    TutoringExample,
}

impl Format {
    /// The taxonomy §3.3 slug — also the record directory name and the
    /// schema filename stem.
    pub fn slug(self) -> &'static str {
        match self {
            Self::Mcq => "mcq",
            Self::CaseVignette => "case_vignette",
            Self::TutoringExample => "tutoring_example",
        }
    }

    /// All formats, for directory matching.
    pub fn all() -> [Self; 3] {
        [Self::Mcq, Self::CaseVignette, Self::TutoringExample]
    }
}

/// A single validation failure on one record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Path to the record file, relative where possible.
    pub path: PathBuf,
    /// 1-based line number inside a `.jsonl` shard, when the record came
    /// from one.
    pub line: Option<usize>,
    /// The record's `id` field, or `<unreadable>` when it could not be read.
    pub record_id: String,
    /// The check that failed.
    pub check: Check,
    /// The rule or path that failed, plus a human-readable detail.
    pub detail: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let loc = match self.line {
            Some(n) => format!("{}:{n}", self.path.display()),
            None => self.path.display().to_string(),
        };
        write!(
            f,
            "{loc}: [{}] record {}: {}",
            self.check, self.record_id, self.detail
        )
    }
}

/// A record loaded from disk, with its format, parsed JSON, and raw text.
#[derive(Debug)]
pub struct RecordFile {
    /// Path to the file as loaded (relative to the corpus root when loaded
    /// via [`load_corpus`]).
    pub path: PathBuf,
    /// 1-based line number inside a `.jsonl` shard, or `None` for
    /// single-document `.json` files.
    pub line: Option<usize>,
    /// The record format, from the format-slug directory component.
    pub format: Format,
    /// The parsed JSON instance.
    pub value: serde_json::Value,
    /// The original record text — cross-field deserialization runs on this
    /// so duplicate JSON fields still fail the way they would for any
    /// consumer of the file.
    pub raw: String,
}

impl RecordFile {
    /// The record's `id` field, or a placeholder when unreadable.
    pub fn record_id(&self) -> String {
        self.value
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("<no-id>")
            .to_owned()
    }
}

/// Compiled validators for the three formats, built from
/// [`schema_files`] — the generated schemas are the single source of truth.
pub struct Validators {
    /// One compiled validator per format.
    pub by_format: HashMap<Format, jsonschema::Validator>,
}

impl Validators {
    /// Compile the generated schemas.
    pub fn compile() -> Self {
        let by_format = schema_files()
            .into_iter()
            .filter_map(|f| {
                let slug = f.filename.split('.').next()?;
                let format = Format::all().into_iter().find(|fmt| fmt.slug() == slug)?;
                let validator =
                    jsonschema::validator_for(&f.schema).expect("generated schema compiles");
                Some((format, validator))
            })
            .collect();
        Self { by_format }
    }
}

/// Load the approved anchor source ids from `docs/sources.json` — contract
/// §4: only registry ids may be cited. An absent or unreadable registry
/// yields an empty set, which makes the `anchor-presence` check flag any
/// record above `unverified` (a loud failure, not a silent pass).
pub fn load_registry(root: &Path) -> HashSet<String> {
    let Ok(text) = fs::read_to_string(root.join("docs/sources.json")) else {
        return HashSet::new();
    };
    serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v["sources"].as_array().map(|s| {
                s.iter()
                    .filter_map(|e| e["source_id"].as_str().map(str::to_owned))
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// Collect the record files the gate validates: every `*.json` and `*.jsonl`
/// under `data/` (including `data/gold/`) and every `valid-*.json` fixture
/// under `fixtures/` (at any depth, so a root-level `fixtures/valid-*.json`
/// is still seen). Missing directories yield no files.
pub fn record_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_json(root.join("data"), &mut files);
    collect_matching(root.join("fixtures"), "valid-", &mut files);
    files.sort();
    files
}

/// Every `*.json`/`*.jsonl` under a directory, recursively — the gold-set
/// test uses this on `data/gold/`.
pub fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_json(dir.to_path_buf(), &mut files);
    files.sort();
    files
}

fn is_record_file(path: &Path, prefix: Option<&str>) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if let Some(prefix) = prefix
        && !name.starts_with(prefix)
    {
        return false;
    }
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("json") | Some("jsonl")
    )
}

fn collect_json(dir: PathBuf, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Skip symlinked directories — following one that points at an
        // ancestor would recurse forever.
        if entry.file_type().is_ok_and(|t| t.is_symlink()) {
            continue;
        }
        if path.is_dir() {
            collect_json(path, out);
        } else if is_record_file(&path, None) {
            out.push(path);
        }
    }
}

fn collect_matching(dir: PathBuf, prefix: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_symlink()) {
            continue;
        }
        if path.is_dir() {
            collect_matching(path, prefix, out);
        } else if is_record_file(&path, Some(prefix)) {
            out.push(path);
        }
    }
}

/// The format slug carried by a record path: the first directory component
/// equal to a format slug. Always called on the corpus-relative display
/// path (`data/mcq/…`, `fixtures/case_vignette/…`, `data/gold/tutoring_example/…`)
/// so ancestor directories outside the corpus cannot misclassify records.
pub fn format_of(path: &Path) -> Option<Format> {
    path.components().find_map(|c| {
        Format::all()
            .into_iter()
            .find(|f| Some(f.slug()) == c.as_os_str().to_str())
    })
}

/// Load and parse the corpus, collecting findings for unreadable or
/// unclassifiable files instead of aborting.
pub fn load_corpus(root: &Path) -> (Vec<RecordFile>, Vec<Finding>) {
    let root = root.to_path_buf();
    load_paths(record_files(&root), move |p| {
        p.strip_prefix(&root).unwrap_or(p).to_path_buf()
    })
}

/// Load and parse an explicit list of record files — used by the corpus
/// loader and by tests that point the gate at a specific file set.
/// `display` maps each loaded path to the path reported in findings and is
/// also the path `format_of` reads, so it should be corpus-relative.
pub fn load_paths(
    paths: Vec<PathBuf>,
    display: impl Fn(&Path) -> PathBuf,
) -> (Vec<RecordFile>, Vec<Finding>) {
    let mut records = Vec::new();
    let mut findings = Vec::new();
    for path in paths {
        let display_path = display(&path);
        let is_jsonl = path.extension().is_some_and(|e| e == "jsonl");
        let Some(format) = format_of(&display_path) else {
            findings.push(Finding {
                path: display_path,
                line: None,
                record_id: "<no-id>".into(),
                check: Check::Schema,
                detail: "record lives outside a format directory \
                         (mcq/ case_vignette/ tutoring_example/)"
                    .into(),
            });
            continue;
        };
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                findings.push(Finding {
                    path: display_path,
                    line: None,
                    record_id: "<unreadable>".into(),
                    check: Check::Schema,
                    detail: format!("cannot read file: {e}"),
                });
                continue;
            }
        };
        if is_jsonl {
            load_jsonl(&text, &display_path, format, &mut records, &mut findings);
        } else {
            push_record(
                &mut records,
                &mut findings,
                PendingRecord {
                    path: &display_path,
                    line: None,
                    format,
                    raw: &text,
                },
            );
        }
    }
    (records, findings)
}

/// Load each non-blank line of a `.jsonl` shard as its own record; blank
/// lines are findings (a malformed shard must not pass silently).
fn load_jsonl(
    text: &str,
    display_path: &Path,
    format: Format,
    records: &mut Vec<RecordFile>,
    findings: &mut Vec<Finding>,
) {
    for (idx, raw) in text.lines().enumerate() {
        let line = idx + 1;
        if raw.trim().is_empty() {
            findings.push(Finding {
                path: display_path.to_path_buf(),
                line: Some(line),
                record_id: "<unreadable>".into(),
                check: Check::Schema,
                detail: "blank line in JSONL shard".into(),
            });
            continue;
        }
        push_record(
            records,
            findings,
            PendingRecord {
                path: display_path,
                line: Some(line),
                format,
                raw,
            },
        );
    }
}

/// One parsed-but-unvalidated record found by [`load_paths`].
struct PendingRecord<'a> {
    path: &'a Path,
    line: Option<usize>,
    format: Format,
    raw: &'a str,
}

fn push_record(records: &mut Vec<RecordFile>, findings: &mut Vec<Finding>, pending: PendingRecord) {
    match serde_json::from_str(pending.raw) {
        Ok(value) => records.push(RecordFile {
            path: pending.path.to_path_buf(),
            line: pending.line,
            format: pending.format,
            value,
            raw: pending.raw.to_owned(),
        }),
        Err(e) => findings.push(Finding {
            path: pending.path.to_path_buf(),
            line: pending.line,
            record_id: "<unreadable>".into(),
            check: Check::Schema,
            detail: format!("invalid JSON: {e}"),
        }),
    }
}

mod checks;
pub mod originality;

pub use checks::run_checks;
