use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;

use anyhow::bail;
use serde::{Deserialize, Serialize};

use crate::RecordFile;

/// Inputs for one originality audit run.
pub struct AuditOptions {
    /// Repository/corpus root used for report-relative paths.
    pub root: PathBuf,
    /// Record files or directories to audit.
    pub records: Vec<PathBuf>,
    /// JSONL external reference corpus.
    pub references: PathBuf,
    /// Optional split assignments JSON (`{"assignments":{"id":"train"}}`).
    pub splits: Option<PathBuf>,
    /// Optional human-review dispositions JSON.
    pub dispositions: Option<PathBuf>,
    /// External overlap review threshold.
    pub overlap_threshold: f64,
    /// Internal duplicate review threshold.
    pub dedup_threshold: f64,
    /// Same-passage contamination grouping threshold.
    pub contamination_threshold: f64,
}

impl AuditOptions {
    /// Validate thresholds once, before any comparisons.
    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        for (name, value) in [
            ("overlap", self.overlap_threshold),
            ("dedup", self.dedup_threshold),
            ("contamination", self.contamination_threshold),
        ] {
            if !(value > 0.0 && value <= 1.0) {
                bail!("{name} threshold must be in (0, 1], got {value}");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ReferenceLine {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) text: String,
    pub(crate) source_url: String,
    pub(crate) license: String,
    pub(crate) provenance: String,
}

#[derive(Debug)]
pub(crate) struct ReferenceItem {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) source_url: String,
    pub(crate) license: String,
    pub(crate) provenance: String,
    pub(crate) text_sha256: String,
    pub(crate) shingles: HashSet<u64>,
    pub(crate) line: usize,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SplitFile {
    pub(crate) assignments: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DispositionFile {
    #[serde(default)]
    pub(crate) dispositions: Vec<Disposition>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct Disposition {
    pub flag_id: String,
    pub decision: String,
    pub reviewer_run_id: String,
    pub rationale: String,
}

#[derive(Debug)]
pub(crate) struct AuditRecord {
    pub(crate) record: RecordFile,
    pub(crate) cell: String,
    pub(crate) prompt_sha256: String,
    pub(crate) full_sha256: String,
    pub(crate) prompt_shingles: HashSet<u64>,
    pub(crate) anchors: BTreeSet<String>,
}

#[derive(Debug, Serialize)]
pub struct InputSummary {
    pub records: usize,
    pub reference_items: usize,
    pub record_corpus_sha256: String,
    pub reference_corpus_sha256: String,
    pub splits_sha256: Option<String>,
    pub dispositions_sha256: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CountSummary {
    pub external_pairs_screened: u64,
    pub internal_pairs_screened: u64,
    pub external_overlap_flags: usize,
    pub internal_dedup_flags: usize,
    pub split_violation_flags: usize,
    pub contamination_groups: usize,
    pub needs_review: usize,
    pub dispositions_applied: usize,
    pub release_blocking_flags: usize,
}

#[derive(Debug, Serialize)]
pub struct RecordSummary {
    pub record_id: String,
    pub path: String,
    pub line: Option<usize>,
    pub format: String,
    pub domain: String,
    pub tier: String,
    pub coverage_cell: String,
    pub prompt_sha256: String,
    pub record_text_sha256: String,
    pub anchor_passages: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ReferenceSummary {
    pub reference_id: String,
    pub kind: String,
    pub source_url: String,
    pub license: String,
    pub provenance: String,
    pub text_sha256: String,
    pub line: usize,
}

#[derive(Debug, Serialize)]
pub struct AuditFlag {
    pub flag_id: String,
    pub kind: String,
    pub record_ids: Vec<String>,
    pub reference_id: Option<String>,
    pub coverage_cell: Option<String>,
    pub anchor_passage_overlap: Vec<String>,
    pub containment: f64,
    pub threshold: f64,
    pub status: String,
    pub disposition: Option<Disposition>,
    pub release_blocking: bool,
}

#[derive(Debug, Serialize)]
pub struct ContaminationGroup {
    pub group_id: String,
    pub record_ids: Vec<String>,
    pub shared_anchor_passages: Vec<String>,
    pub max_containment: f64,
    pub split_assignments: BTreeMap<String, String>,
}

#[derive(Debug, Serialize)]
pub struct SplitSummary {
    pub status: String,
    pub assignments: usize,
    pub violations: usize,
}

/// The deterministic originality audit report.
#[derive(Debug, Serialize)]
pub struct AuditReport {
    pub report_version: u32,
    pub inputs: InputSummary,
    pub thresholds: BTreeMap<String, f64>,
    pub counts: CountSummary,
    pub records: Vec<RecordSummary>,
    pub reference_items: Vec<ReferenceSummary>,
    pub flags: Vec<AuditFlag>,
    pub contamination_groups: Vec<ContaminationGroup>,
    pub split_audit: SplitSummary,
    pub unused_disposition_ids: Vec<String>,
    pub release_blocking_record_ids: Vec<String>,
}

impl AuditReport {
    /// Whether this report blocks the candidate as audited.
    pub fn release_blocking(&self) -> bool {
        self.counts.release_blocking_flags > 0
    }
}
