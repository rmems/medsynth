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
            for (idx, raw) in text.lines().enumerate() {
                let line = idx + 1;
                if raw.trim().is_empty() {
                    continue;
                }
                push_record(
                    &mut records,
                    &mut findings,
                    &display_path,
                    Some(line),
                    format,
                    raw,
                );
            }
        } else {
            push_record(
                &mut records,
                &mut findings,
                &display_path,
                None,
                format,
                &text,
            );
        }
    }
    (records, findings)
}

fn push_record(
    records: &mut Vec<RecordFile>,
    findings: &mut Vec<Finding>,
    display_path: &Path,
    line: Option<usize>,
    format: Format,
    raw: &str,
) {
    match serde_json::from_str(raw) {
        Ok(value) => records.push(RecordFile {
            path: display_path.to_path_buf(),
            line,
            format,
            value,
            raw: raw.to_owned(),
        }),
        Err(e) => findings.push(Finding {
            path: display_path.to_path_buf(),
            line,
            record_id: "<unreadable>".into(),
            check: Check::Schema,
            detail: format!("invalid JSON: {e}"),
        }),
    }
}

/// Run every check over the corpus and return all findings. `registry` is
/// the approved source-id set from [`load_registry`].
pub fn run_checks(
    records: &[RecordFile],
    validators: &Validators,
    registry: &HashSet<String>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    duplicate_id_check(records, &mut findings);
    for record in records {
        schema_check(record, validators, &mut findings);
        key_rationale_check(record, &mut findings);
        anchor_presence_check(record, registry, &mut findings);
    }
    originality::screen(records, &mut findings);
    findings.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.check.name().cmp(b.check.name()))
            .then(a.detail.cmp(&b.detail))
    });
    findings
}

/// Record ids must be unique across the corpus — a duplicate makes the
/// identity ambiguous for consumers and for finding reports.
fn duplicate_id_check(records: &[RecordFile], findings: &mut Vec<Finding>) {
    let mut seen: HashMap<String, &RecordFile> = HashMap::new();
    for record in records {
        let id = record.record_id();
        if id == "<no-id>" {
            continue;
        }
        if let Some(first) = seen.get(&id) {
            findings.push(Finding {
                path: record.path.clone(),
                line: record.line,
                record_id: id.clone(),
                check: Check::Schema,
                detail: format!(
                    "duplicate record id (first seen at {})",
                    first.path.display()
                ),
            });
        } else {
            seen.insert(id, record);
        }
    }
}

/// `schema` — JSON Schema validation plus deserialize (of the raw record
/// text, so duplicate fields fail) + cross-field `validate()`.
fn schema_check(record: &RecordFile, validators: &Validators, findings: &mut Vec<Finding>) {
    let mut push = |detail: String| {
        findings.push(Finding {
            path: record.path.clone(),
            line: record.line,
            record_id: record.record_id(),
            check: Check::Schema,
            detail,
        });
    };
    let Some(validator) = validators.by_format.get(&record.format) else {
        push(format!("no schema for format {}", record.format.slug()));
        return;
    };
    let schema_errors: Vec<String> = validator
        .iter_errors(&record.value)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    if !schema_errors.is_empty() {
        for detail in schema_errors {
            push(detail);
        }
        return;
    }
    // Deserialization failure names the serde path; a validate() failure
    // lists the cross-field rules broken.
    match record.format {
        Format::Mcq => cross_detail::<McqRecord>(record, findings),
        Format::CaseVignette => cross_detail::<CaseVignetteRecord>(record, findings),
        Format::TutoringExample => cross_detail::<TutoringExampleRecord>(record, findings),
    }
}

fn cross_detail<T>(record: &RecordFile, findings: &mut Vec<Finding>)
where
    T: serde::de::DeserializeOwned + CrossValidate,
{
    let mut push = |detail: String| {
        findings.push(Finding {
            path: record.path.clone(),
            line: record.line,
            record_id: record.record_id(),
            check: Check::Schema,
            detail,
        });
    };
    match serde_json::from_str::<T>(&record.raw) {
        Err(e) => push(format!("record does not deserialize: {e}")),
        Ok(r) => {
            if let Err(errors) = r.validate() {
                for e in errors {
                    push(format!("cross-field rule: {e}"));
                }
            }
        }
    }
}

/// Lets `schema_check` call each record type's `validate` generically.
pub trait CrossValidate {
    /// Cross-field `validate` from `medsynth-schema`.
    fn validate(&self) -> Result<(), Vec<ValidationError>>;
}
impl CrossValidate for McqRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        McqRecord::validate(self)
    }
}
impl CrossValidate for CaseVignetteRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        CaseVignetteRecord::validate(self)
    }
}
impl CrossValidate for TutoringExampleRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        TutoringExampleRecord::validate(self)
    }
}

/// `key-rationale` — rationale present, non-empty, referencing the keyed
/// answer; answer fields non-empty; `AnchoredClaim` anchors non-empty.
fn key_rationale_check(record: &RecordFile, findings: &mut Vec<Finding>) {
    match record.format {
        Format::Mcq => key_rationale_mcq(record, findings),
        Format::CaseVignette => key_rationale_vignette(record, findings),
        Format::TutoringExample => key_rationale_tutoring(record, findings),
    }
}

fn push_lint(record: &RecordFile, findings: &mut Vec<Finding>, detail: String) {
    findings.push(Finding {
        path: record.path.clone(),
        line: record.line,
        record_id: record.record_id(),
        check: Check::KeyRationale,
        detail,
    });
}

fn key_rationale_mcq(record: &RecordFile, findings: &mut Vec<Finding>) {
    let v = &record.value;
    let Some(rationale) = v.get("rationale") else {
        push_lint(record, findings, "rationale field missing".into());
        return;
    };
    for field in ["key_explanation", "strongest_distractor"] {
        let claim = &rationale[field];
        if claim["text"].as_str().is_none_or(|t| t.trim().is_empty()) {
            push_lint(record, findings, format!("rationale.{field}.text is empty"));
        }
        if claim["anchors"].as_array().is_none_or(|a| a.is_empty()) {
            push_lint(
                record,
                findings,
                format!("rationale.{field} has no anchors"),
            );
        }
    }
    key_references_option(record, v, rationale, findings);
}

/// The rationale must reference the keyed answer. Cheap deterministic
/// proxy: the keyed option must share strictly more content words
/// (lowercased tokens, len ≥ 4) with `key_explanation` than every other
/// option — a wrong key leaves the rationale explaining another option,
/// and a tie gives no evidence the rationale favors the keyed answer.
fn key_references_option(
    record: &RecordFile,
    v: &serde_json::Value,
    rationale: &serde_json::Value,
    findings: &mut Vec<Finding>,
) {
    let (Some(key), Some(options), Some(explanation)) = (
        v.get("key").and_then(|k| k.as_str()),
        v.get("options").and_then(|o| o.as_array()),
        rationale["key_explanation"]["text"].as_str(),
    ) else {
        return; // missing fields are the schema check's findings
    };
    let explained = words(explanation);
    let overlap = |label: &str| -> usize {
        options
            .iter()
            .find(|o| o["label"].as_str() == Some(label))
            .map(|o| words(o["text"].as_str().unwrap_or("")))
            .map(|w| explained.intersection(&w).count())
            .unwrap_or(0)
    };
    let own = overlap(key);
    let best_other = options
        .iter()
        .filter_map(|o| o["label"].as_str())
        .filter(|l| *l != key)
        .map(overlap)
        .max()
        .unwrap_or(0);
    if own <= best_other {
        let why = if own == best_other {
            "does not uniquely explain"
        } else {
            "is not the option the rationale explains"
        };
        push_lint(
            record,
            findings,
            format!(
                "key `{key}` {why} the keyed answer \
                 (keyed overlap {own}, best other {best_other})"
            ),
        );
    }
}

fn key_rationale_vignette(record: &RecordFile, findings: &mut Vec<Finding>) {
    let v = &record.value;
    for (kind, claims) in [("questions", "answer"), ("teaching_points", "")] {
        let Some(items) = v.get(kind).and_then(|q| q.as_array()) else {
            continue;
        };
        for (i, item) in items.iter().enumerate() {
            let claim = if claims.is_empty() {
                item
            } else {
                &item[claims]
            };
            if claim["text"].as_str().is_none_or(|t| t.trim().is_empty()) {
                let label = if claims.is_empty() {
                    format!("{kind}[{i}].text is empty")
                } else {
                    format!("{kind}[{i}].{claims}.text is empty")
                };
                push_lint(record, findings, label);
            }
            if claim["anchors"].as_array().is_none_or(|a| a.is_empty()) {
                let label = if claims.is_empty() {
                    format!("{kind}[{i}] has no anchors")
                } else {
                    format!("{kind}[{i}].{claims} has no anchors")
                };
                push_lint(record, findings, label);
            }
        }
    }
}

fn key_rationale_tutoring(record: &RecordFile, findings: &mut Vec<Finding>) {
    let response = &record.value["tutor_response"];
    if response["text"]
        .as_str()
        .is_none_or(|t| t.trim().is_empty())
    {
        push_lint(record, findings, "tutor_response.text is empty".into());
    }
}

/// Content words for the key/rationale overlap check: lowercased word
/// tokens of length ≥ 4.
fn words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 4)
        .map(|w| w.to_lowercase())
        .collect()
}

/// Rank of a [`VerificationStatus`] in the provenance ordering —
/// `unverified` < `key-checked` < `anchor-checked` < `admitted`, the
/// declaration order of the `medsynth-schema` enum.
fn status_rank(value: &serde_json::Value) -> Option<u8> {
    let s = value.get("verification_status")?.as_str()?;
    [
        VerificationStatus::Unverified,
        VerificationStatus::KeyChecked,
        VerificationStatus::AnchorChecked,
        VerificationStatus::Admitted,
    ]
    .iter()
    .position(|v| {
        serde_json::to_value(v)
            .ok()
            .and_then(|j| j.as_str().map(str::to_owned))
            .as_deref()
            == Some(s)
    })
    .map(|p| p as u8)
}

/// `anchor-presence` — a record above `unverified` must carry at least one
/// anchor citing a source id in the approved registry (contract §4).
fn anchor_presence_check(
    record: &RecordFile,
    registry: &HashSet<String>,
    findings: &mut Vec<Finding>,
) {
    let Some(rank) = status_rank(&record.value) else {
        return; // missing/garbage status is the schema check's finding
    };
    if rank > 0 && count_registry_anchors(&record.value, registry) == 0 {
        findings.push(Finding {
            path: record.path.clone(),
            line: record.line,
            record_id: record.record_id(),
            check: Check::AnchorPresence,
            detail: format!(
                "verification_status `{}` is above unverified but the record \
                 carries no anchors into the approved source registry",
                record.value["verification_status"].as_str().unwrap_or("?")
            ),
        });
    }
}

/// Count anchor objects anywhere in the record whose `source_id` is in the
/// approved registry — an anchor to an unlisted source is no anchor
/// (contract §4).
fn count_registry_anchors(value: &serde_json::Value, registry: &HashSet<String>) -> usize {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                if k == "anchors" {
                    v.as_array().map_or(0, |anchors| {
                        anchors
                            .iter()
                            .filter(|a| {
                                a["source_id"]
                                    .as_str()
                                    .is_some_and(|id| registry.contains(id))
                            })
                            .count()
                    })
                } else {
                    count_registry_anchors(v, registry)
                }
            })
            .sum(),
        serde_json::Value::Array(items) => items
            .iter()
            .map(|i| count_registry_anchors(i, registry))
            .sum(),
        _ => 0,
    }
}

/// The smoke originality screen.
///
/// Word **5-gram** shingles are built from each record's text fields — every
/// string under keys `text`, `presentation`, `question`, and every string
/// inside `findings`/`options` arrays — lowercased, alphanumeric tokens,
/// hashed with a stable FNV-1a so results are identical across toolchains
/// and platforms. A pair of records is flagged when either record's shingle
/// set is contained in the other's at a fraction ≥ **0.6**: a copied or
/// near-copied record shares essentially all of its shingles with its
/// source, while two records merely on the same topic share few 5-grams.
/// Both records of an offending pair are reported. This is the smoke
/// screen only; the full originality and contamination audit is #11.
pub mod originality {
    use super::*;

    /// Word n-gram size for the overlap shingles.
    pub const NGRAM: usize = 5;
    /// Containment fraction at or above which a record pair is flagged as
    /// likely duplicates.
    pub const THRESHOLD: f64 = 0.6;

    /// The text fields the screen reads: string values under these keys,
    /// plus all string members of `findings` and `options` arrays.
    const TEXT_KEYS: [&str; 3] = ["text", "presentation", "question"];
    const TEXT_ARRAY_KEYS: [&str; 2] = ["findings", "options"];

    /// All record text the screen sees, concatenated.
    pub fn record_text(value: &serde_json::Value) -> String {
        let mut out = String::new();
        collect(value, None, &mut out);
        out
    }

    fn collect(value: &serde_json::Value, key: Option<&str>, out: &mut String) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    collect(v, Some(k), out);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    collect(item, key, out);
                }
            }
            serde_json::Value::String(s)
                if key.is_some_and(|k| TEXT_KEYS.contains(&k))
                    || key.is_some_and(|k| TEXT_ARRAY_KEYS.contains(&k)) =>
            {
                out.push_str(s);
                out.push(' ');
            }
            _ => {}
        }
    }

    /// FNV-1a over the token — stable across Rust versions and platforms,
    /// unlike `DefaultHasher`.
    fn fnv1a(token: &[u8], seed: u64) -> u64 {
        token.iter().fold(seed, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    fn shingles(text: &str) -> HashSet<u64> {
        let tokens: Vec<String> = text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| w.to_lowercase())
            .collect();
        tokens
            .windows(NGRAM)
            .map(|w| {
                w.iter()
                    .fold(0xcbf2_9ce4_8422_2325, |h, t| fnv1a(t.as_bytes(), h))
            })
            .collect()
    }

    /// Flag record pairs whose shingle containment — |shared| / |smaller
    /// set| — reaches [`THRESHOLD`] in either direction. Both records of a
    /// pair are reported.
    pub fn screen(records: &[RecordFile], findings: &mut Vec<Finding>) {
        let sets: Vec<HashSet<u64>> = records
            .iter()
            .map(|r| shingles(&record_text(&r.value)))
            .collect();
        let mut flagged: HashMap<usize, usize> = HashMap::new(); // record -> first offender
        for i in 0..sets.len() {
            for j in (i + 1)..sets.len() {
                let (a, b) = (&sets[i], &sets[j]);
                if a.is_empty() || b.is_empty() {
                    continue;
                }
                let shared = a.intersection(b).count();
                let smaller = a.len().min(b.len()) as f64;
                if shared as f64 / smaller >= THRESHOLD {
                    flagged.entry(i).or_insert(j);
                    flagged.entry(j).or_insert(i);
                }
            }
        }
        for (i, j) in flagged {
            findings.push(Finding {
                path: records[i].path.clone(),
                line: records[i].line,
                record_id: records[i].record_id(),
                check: Check::Originality,
                detail: format!(
                    "text near-duplicates {} (5-gram containment >= {THRESHOLD})",
                    records[j].path.display()
                ),
            });
        }
    }
}
