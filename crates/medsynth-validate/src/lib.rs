//! The medsynth data-validation gate.
//!
//! One binary runs every check over the record corpus — every `*.json`
//! record under `data/` (including `data/gold/`) and every `valid-*.json`
//! fixture under `fixtures/` — and reports every failure it finds before
//! exiting nonzero. The checks:
//!
//! * **`schema`** — JSON Schema validation against the schemas generated
//!   from `medsynth-schema`'s record types (the single source of truth),
//!   plus a required deserialize into the typed record and its cross-field
//!   `validate()`.
//! * **`key-rationale`** — rationale present and non-empty and referencing
//!   the keyed answer; answer/response fields non-empty on the formats
//!   that carry them; anchor fields present and non-empty where the format
//!   carries them (`AnchoredClaim` fields).
//! * **`anchor-presence`** — no record whose `verification_status` is above
//!   `unverified` may lack anchors entirely. The status ordering is the
//!   declaration order of `medsynth-schema`'s [`VerificationStatus`]
//!   (`unverified` < `key-checked` < `anchor-checked` < `admitted`); this
//!   gate never advances any record's status.
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
    /// JSON Schema, deserialization, or cross-field `validate()` failure.
    Schema,
    /// Rationale/answer missing, empty, or not referencing the keyed answer.
    KeyRationale,
    /// A record above `unverified` lacks anchors entirely.
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
    /// The record's `id` field, or `<unreadable>` when it could not be read.
    pub record_id: String,
    /// The check that failed.
    pub check: Check,
    /// The rule or path that failed, plus a human-readable detail.
    pub detail: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}: [{}] record {}: {}",
            self.path.display(),
            self.check,
            self.record_id,
            self.detail
        )
    }
}

/// A record file loaded from disk, with its format and parsed JSON.
#[derive(Debug)]
pub struct RecordFile {
    /// Path to the file as loaded.
    pub path: PathBuf,
    /// The record format, from the format-slug directory component.
    pub format: Format,
    /// The parsed JSON instance.
    pub value: serde_json::Value,
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

/// Every `*.json` under a directory, recursively — the gold-set test uses
/// this on `data/gold/`.
pub fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_json(dir.to_path_buf(), &mut files);
    files.sort();
    files
}

/// Collect the record files the gate validates: every `*.json` under
/// `data/` (including `data/gold/`) and every `valid-*.json` fixture under
/// `fixtures/`. Directories that do not exist yield no files.
pub fn record_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_json(root.join("data"), &mut files);
    if let Ok(entries) = fs::read_dir(root.join("fixtures")) {
        for entry in entries.flatten() {
            collect_matching(entry.path(), "valid-", &mut files);
        }
    }
    files.sort();
    files
}

fn collect_json(dir: PathBuf, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_json(path, out);
        } else if path.extension().is_some_and(|e| e == "json") {
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
        if path.is_dir() {
            collect_matching(path, prefix, out);
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".json"))
        {
            out.push(path);
        }
    }
}

/// The format slug carried by a record path: the first directory component
/// equal to a format slug (`data/mcq/…`, `fixtures/case_vignette/…`,
/// `data/gold/tutoring_example/…`).
pub fn format_of(path: &Path) -> Option<Format> {
    path.components().find_map(|c| {
        Format::all()
            .into_iter()
            .find(|f| Some(f.slug()) == c.as_os_str().to_str())
    })
}

/// Load and parse the corpus, collecting findings for unreadable or
/// unclassifiable files instead of aborting. Returns the records that
/// parsed plus the findings produced while loading.
pub fn load_corpus(root: &Path) -> (Vec<RecordFile>, Vec<Finding>) {
    let root = root.to_path_buf();
    load_paths(record_files(&root), move |p| {
        p.strip_prefix(&root).unwrap_or(p).to_path_buf()
    })
}

/// Load and parse an explicit list of record files — used by the corpus
/// loader and by tests that point the gate at a specific file set.
/// `display` maps each loaded path to the path reported in findings.
pub fn load_paths(
    paths: Vec<PathBuf>,
    display: impl Fn(&Path) -> PathBuf,
) -> (Vec<RecordFile>, Vec<Finding>) {
    let mut records = Vec::new();
    let mut findings = Vec::new();
    for path in paths {
        let display_path = display(&path);
        let Some(format) = format_of(&path) else {
            findings.push(Finding {
                path: display_path,
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
                    record_id: "<unreadable>".into(),
                    check: Check::Schema,
                    detail: format!("cannot read file: {e}"),
                });
                continue;
            }
        };
        match serde_json::from_str(&text) {
            Ok(value) => records.push(RecordFile {
                path: display_path,
                format,
                value,
            }),
            Err(e) => findings.push(Finding {
                path: display_path,
                record_id: "<unreadable>".into(),
                check: Check::Schema,
                detail: format!("invalid JSON: {e}"),
            }),
        }
    }
    (records, findings)
}

/// Run every check over the corpus and return all findings.
pub fn run_checks(records: &[RecordFile], validators: &Validators) -> Vec<Finding> {
    let mut findings = Vec::new();
    for record in records {
        schema_check(record, validators, &mut findings);
        key_rationale_check(record, &mut findings);
        anchor_presence_check(record, &mut findings);
    }
    originality::screen(records, &mut findings);
    findings.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.check.name().cmp(b.check.name()))
            .then(a.detail.cmp(&b.detail))
    });
    findings
}

/// `schema` — JSON Schema validation plus deserialize + cross-field
/// `validate()` for the record's format.
fn schema_check(record: &RecordFile, validators: &Validators, findings: &mut Vec<Finding>) {
    let push = |findings: &mut Vec<Finding>, detail: String| {
        findings.push(Finding {
            path: record.path.clone(),
            record_id: record.record_id(),
            check: Check::Schema,
            detail,
        });
    };
    let Some(validator) = validators.by_format.get(&record.format) else {
        push(
            findings,
            format!("no schema for format {}", record.format.slug()),
        );
        return;
    };
    let schema_errors: Vec<String> = validator
        .iter_errors(&record.value)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    if !schema_errors.is_empty() {
        for detail in schema_errors {
            push(findings, detail);
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
            record_id: record.record_id(),
            check: Check::Schema,
            detail,
        });
    };
    match serde_json::from_value::<T>(record.value.clone()) {
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
    let push = |findings: &mut Vec<Finding>, detail: &str| {
        findings.push(Finding {
            path: record.path.clone(),
            record_id: record.record_id(),
            check: Check::KeyRationale,
            detail: detail.to_owned(),
        });
    };
    match record.format {
        Format::Mcq => {
            let v = &record.value;
            let Some(rationale) = v.get("rationale") else {
                push(findings, "rationale field missing");
                return;
            };
            for field in ["key_explanation", "strongest_distractor"] {
                let claim = &rationale[field];
                if claim["text"].as_str().is_none_or(|t| t.trim().is_empty()) {
                    push(findings, &format!("rationale.{field}.text is empty"));
                }
                if claim["anchors"].as_array().is_none_or(|a| a.is_empty()) {
                    push(findings, &format!("rationale.{field} has no anchors"));
                }
            }
            // The rationale must reference the keyed answer. Cheap proxy:
            // the keyed option must share more content words (len ≥ 4,
            // lowercased) with `key_explanation` than any other option —
            // a wrong key leaves the rationale explaining another option.
            if let (Some(key), Some(options), Some(explanation)) = (
                v.get("key").and_then(|k| k.as_str()),
                v.get("options").and_then(|o| o.as_array()),
                rationale["key_explanation"]["text"].as_str(),
            ) {
                let explained = words(explanation);
                let keyed_overlap = |label: &str| -> usize {
                    options
                        .iter()
                        .find(|o| o["label"].as_str() == Some(label))
                        .map(|o| words(o["text"].as_str().unwrap_or("")))
                        .map(|w| explained.intersection(&w).count())
                        .unwrap_or(0)
                };
                let own = keyed_overlap(key);
                let best_other = options
                    .iter()
                    .filter_map(|o| o["label"].as_str())
                    .filter(|l| *l != key)
                    .map(keyed_overlap)
                    .max()
                    .unwrap_or(0);
                if own < best_other {
                    push(
                        findings,
                        &format!(
                            "key `{key}` is not the option the rationale explains \
                             (keyed overlap {own}, best other {best_other})"
                        ),
                    );
                }
            }
        }
        Format::CaseVignette => {
            let v = &record.value;
            if let Some(questions) = v.get("questions").and_then(|q| q.as_array()) {
                for (i, q) in questions.iter().enumerate() {
                    let answer = &q["answer"];
                    if answer["text"].as_str().is_none_or(|t| t.trim().is_empty()) {
                        push(findings, &format!("questions[{i}].answer.text is empty"));
                    }
                    if answer["anchors"].as_array().is_none_or(|a| a.is_empty()) {
                        push(findings, &format!("questions[{i}].answer has no anchors"));
                    }
                }
            }
            if let Some(points) = v.get("teaching_points").and_then(|p| p.as_array()) {
                for (i, p) in points.iter().enumerate() {
                    if p["text"].as_str().is_none_or(|t| t.trim().is_empty()) {
                        push(findings, &format!("teaching_points[{i}].text is empty"));
                    }
                    if p["anchors"].as_array().is_none_or(|a| a.is_empty()) {
                        push(findings, &format!("teaching_points[{i}] has no anchors"));
                    }
                }
            }
        }
        Format::TutoringExample => {
            let v = &record.value;
            let response = &v["tutor_response"];
            if response["text"]
                .as_str()
                .is_none_or(|t| t.trim().is_empty())
            {
                push(findings, "tutor_response.text is empty");
            }
        }
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
/// anchor anywhere in its fields.
fn anchor_presence_check(record: &RecordFile, findings: &mut Vec<Finding>) {
    let Some(rank) = status_rank(&record.value) else {
        return; // missing/garbage status is the schema check's finding
    };
    if rank > 0 && count_anchors(&record.value) == 0 {
        findings.push(Finding {
            path: record.path.clone(),
            record_id: record.record_id(),
            check: Check::AnchorPresence,
            detail: format!(
                "verification_status `{}` is above unverified but the record \
                 carries no anchors",
                record.value["verification_status"].as_str().unwrap_or("?")
            ),
        });
    }
}

/// Count every anchor object anywhere in the record — any `"anchors"` array
/// member counts.
fn count_anchors(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                if k == "anchors" {
                    v.as_array().map_or(0, |a| a.len())
                } else {
                    count_anchors(v)
                }
            })
            .sum(),
        serde_json::Value::Array(items) => items.iter().map(count_anchors).sum(),
        _ => 0,
    }
}

/// The smoke originality screen.
///
/// Word **5-gram** shingles are built from each record's text fields — every
/// string under keys `text`, `presentation`, `question`, and every string
/// inside `findings`/`options` arrays — lowercased, alphanumeric tokens.
/// A record is flagged when its shingle set's **containment** in another
/// record's set (|shared| / |this record's shingles|) reaches **0.6**: a
/// copied or near-copied record shares essentially all of its shingles with
/// its source, while two records merely on the same topic share few 5-grams.
/// Both records of an offending pair are reported. This is the smoke screen
/// only; the full originality and contamination audit is #11.
pub mod originality {
    use super::*;

    /// Word n-gram size for the overlap shingles.
    pub const NGRAM: usize = 5;
    /// Containment fraction at or above which a record is flagged as a
    /// likely duplicate of another.
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

    fn shingles(text: &str) -> HashSet<u64> {
        let tokens: Vec<String> = text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| w.to_lowercase())
            .collect();
        tokens
            .windows(NGRAM)
            .map(|w| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                for t in w {
                    std::hash::Hash::hash(t, &mut h);
                }
                std::hash::Hasher::finish(&h)
            })
            .collect()
    }

    /// Flag every record whose shingle containment in any other record's
    /// shingle set reaches [`THRESHOLD`].
    pub fn screen(records: &[RecordFile], findings: &mut Vec<Finding>) {
        let sets: Vec<HashSet<u64>> = records
            .iter()
            .map(|r| shingles(&record_text(&r.value)))
            .collect();
        for (i, mine) in sets.iter().enumerate() {
            if mine.is_empty() {
                continue;
            }
            for (j, other) in sets.iter().enumerate() {
                if i == j {
                    continue;
                }
                let shared = mine.intersection(other).count();
                let containment = shared as f64 / mine.len() as f64;
                if containment >= THRESHOLD {
                    findings.push(Finding {
                        path: records[i].path.clone(),
                        record_id: records[i].record_id(),
                        check: Check::Originality,
                        detail: format!(
                            "text near-duplicates {} (5-gram containment {containment:.2} \
                             >= {THRESHOLD})",
                            records[j].path.display()
                        ),
                    });
                    break; // one offender pair per record is enough
                }
            }
        }
    }
}
