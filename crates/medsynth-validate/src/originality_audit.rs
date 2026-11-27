//! Originality and contamination audit (#11).
//!
//! This is the release-candidate audit behind the validator's smoke
//! originality screen: compare item prompts against a rights-documented
//! external reference corpus, detect same-cell internal duplication, and
//! group same-passage near-duplicates so assembly can keep them split-aware.
//! The report carries hashes, metrics, ids, and dispositions — never the
//! reference item text.

mod analyze;
mod common;
mod load;
mod model;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Context, bail};

use analyze::{
    apply_dispositions, corpus_hash, count_kind, cross_split_flags, external_overlap_flags,
    record_summaries, reference_summaries, scan_internal,
};
use load::{load_audit_records, load_dispositions, load_references, load_splits};
use model::{AuditRecord, ReferenceItem};

pub use model::{
    AuditFlag, AuditOptions, AuditReport, ContaminationGroup, ContaminationLink, CountSummary,
    Disposition, InputSummary, RecordSummary, ReferenceSummary, SplitSummary,
};

/// Default prompt-overlap threshold for external contamination review.
pub const DEFAULT_OVERLAP_THRESHOLD: f64 = 0.6;
/// Default same-cell internal duplication threshold.
pub const DEFAULT_DEDUP_THRESHOLD: f64 = 0.35;
/// Default same-anchor contamination grouping threshold.
pub const DEFAULT_CONTAMINATION_THRESHOLD: f64 = 0.35;

struct AuditInputs {
    root: PathBuf,
    records: Vec<AuditRecord>,
    references: Vec<ReferenceItem>,
    reference_hash: String,
    splits: BTreeMap<String, String>,
    splits_hash: Option<String>,
    dispositions: std::collections::HashMap<String, Disposition>,
    dispositions_hash: Option<String>,
}

fn require_complete_splits(
    records: &[AuditRecord],
    splits: &BTreeMap<String, String>,
) -> anyhow::Result<()> {
    let missing: Vec<String> = records
        .iter()
        .map(|record| record.record.record_id())
        .filter(|id| !splits.contains_key(id))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    bail!(
        "split assignments are incomplete for audited records: {}",
        missing.join(", ")
    )
}

fn load_inputs(options: &AuditOptions) -> anyhow::Result<AuditInputs> {
    let root = options
        .root
        .canonicalize()
        .with_context(|| format!("root {} does not exist", options.root.display()))?;
    let records = load_audit_records(&root, &options.records)?;
    let (references, reference_hash) = load_references(&options.references)?;
    let (splits, splits_hash) = load_splits(options.splits.as_ref())?;
    let (dispositions, dispositions_hash) = load_dispositions(options.dispositions.as_ref())?;
    if options.splits.is_some() {
        require_complete_splits(&records, &splits)?;
    }
    Ok(AuditInputs {
        root,
        records,
        references,
        reference_hash,
        splits,
        splits_hash,
        dispositions,
        dispositions_hash,
    })
}

struct ScanOutcome {
    flags: Vec<AuditFlag>,
    contamination_groups: Vec<ContaminationGroup>,
    external_count: usize,
    internal_count: usize,
    split_count: usize,
    internal_pairs: u64,
}

fn scan(options: &AuditOptions, inputs: &AuditInputs) -> ScanOutcome {
    let mut flags = external_overlap_flags(
        &inputs.records,
        &inputs.references,
        options.overlap_threshold,
    );
    let internal = scan_internal(
        &inputs.records,
        options.dedup_threshold,
        options.contamination_threshold,
        &inputs.splits,
    );
    let internal_count = internal.flags.len();
    let contamination_groups = internal.groups;
    flags.extend(internal.flags);
    let split_flags = cross_split_flags(&contamination_groups, options.contamination_threshold);
    let split_count = split_flags.len();
    flags.extend(split_flags);
    ScanOutcome {
        external_count: count_kind(&flags, "external-overlap"),
        flags,
        contamination_groups,
        internal_count,
        split_count,
        internal_pairs: internal.pairs,
    }
}

struct ReviewOutcome {
    unused_disposition_ids: Vec<String>,
    dispositions_applied: usize,
    needs_review: usize,
    release_blocking_flags: usize,
    release_blocking_record_ids: Vec<String>,
}

fn blocking_record_ids(flags: &[AuditFlag]) -> Vec<String> {
    let mut ids: BTreeSet<String> = BTreeSet::new();
    for flag in flags.iter().filter(|flag| flag.release_blocking) {
        ids.extend(flag.record_ids.iter().cloned());
    }
    ids.into_iter().collect()
}

fn review(scan: ScanOutcome, inputs: &AuditInputs) -> (ScanOutcome, ReviewOutcome) {
    let (flags, unused_disposition_ids, dispositions_applied) =
        apply_dispositions(scan.flags, &inputs.dispositions);
    let needs_review = flags
        .iter()
        .filter(|flag| flag.status == "needs-review")
        .count();
    let release_blocking_flags = flags.iter().filter(|flag| flag.release_blocking).count();
    let release_blocking_record_ids = blocking_record_ids(&flags);
    let scan = ScanOutcome { flags, ..scan };
    let review = ReviewOutcome {
        unused_disposition_ids,
        dispositions_applied,
        needs_review,
        release_blocking_flags,
        release_blocking_record_ids,
    };
    (scan, review)
}

fn thresholds(options: &AuditOptions) -> BTreeMap<String, f64> {
    [
        ("external_overlap", options.overlap_threshold),
        ("internal_dedup", options.dedup_threshold),
        ("contamination", options.contamination_threshold),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value))
    .collect()
}

fn input_summary(inputs: &AuditInputs) -> InputSummary {
    InputSummary {
        records: inputs.records.len(),
        reference_items: inputs.references.len(),
        record_corpus_sha256: corpus_hash(&inputs.root, &inputs.records),
        reference_corpus_sha256: inputs.reference_hash.clone(),
        splits_sha256: inputs.splits_hash.clone(),
        dispositions_sha256: inputs.dispositions_hash.clone(),
    }
}

fn count_summary(inputs: &AuditInputs, scan: &ScanOutcome, review: &ReviewOutcome) -> CountSummary {
    CountSummary {
        external_pairs_screened: inputs.records.len() as u64 * inputs.references.len() as u64,
        internal_pairs_screened: scan.internal_pairs,
        external_overlap_flags: scan.external_count,
        internal_dedup_flags: scan.internal_count,
        split_violation_flags: scan.split_count,
        contamination_groups: scan.contamination_groups.len(),
        needs_review: review.needs_review,
        dispositions_applied: review.dispositions_applied,
        release_blocking_flags: review.release_blocking_flags,
    }
}

fn split_summary(options: &AuditOptions, inputs: &AuditInputs, violations: usize) -> SplitSummary {
    let status = if options.splits.is_some() {
        "checked"
    } else {
        "not-run-no-split-assignments"
    };
    SplitSummary {
        status: status.to_owned(),
        assignments: inputs.splits.len(),
        violations,
    }
}

fn build_report(
    options: &AuditOptions,
    inputs: &AuditInputs,
    scan: ScanOutcome,
    review: ReviewOutcome,
) -> AuditReport {
    let counts = count_summary(inputs, &scan, &review);
    let split_audit = split_summary(options, inputs, scan.split_count);
    AuditReport {
        report_version: 1,
        inputs: input_summary(inputs),
        thresholds: thresholds(options),
        counts,
        records: record_summaries(&inputs.root, &inputs.records),
        reference_items: reference_summaries(&inputs.references),
        flags: scan.flags,
        split_audit,
        contamination_groups: scan.contamination_groups,
        unused_disposition_ids: review.unused_disposition_ids,
        release_blocking_record_ids: review.release_blocking_record_ids,
    }
}

/// Run the originality and contamination audit.
pub fn run_audit(options: &AuditOptions) -> anyhow::Result<AuditReport> {
    options.validate()?;
    let inputs = load_inputs(options)?;
    let scan = scan(options, &inputs);
    let (scan, review) = review(scan, &inputs);
    Ok(build_report(options, &inputs, scan, review))
}

/// Serialize a report as stable pretty JSON with a trailing newline.
pub fn report_json(report: &AuditReport) -> anyhow::Result<String> {
    let mut text =
        serde_json::to_string_pretty(report).context("cannot serialize originality report")?;
    text.push('\n');
    Ok(text)
}
