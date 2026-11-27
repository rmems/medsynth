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

use anyhow::{Context, bail};

use analyze::{
    apply_dispositions, corpus_hash, count_kind, cross_split_flags, external_overlap_flags,
    record_summaries, reference_summaries, scan_internal,
};
use load::{load_audit_records, load_dispositions, load_references, load_splits};

pub use model::{
    AuditFlag, AuditOptions, AuditReport, ContaminationGroup, CountSummary, Disposition,
    InputSummary, RecordSummary, ReferenceSummary, SplitSummary,
};

/// Default prompt-overlap threshold for external contamination review.
pub const DEFAULT_OVERLAP_THRESHOLD: f64 = 0.6;
/// Default same-cell internal duplication threshold.
pub const DEFAULT_DEDUP_THRESHOLD: f64 = 0.35;
/// Default same-anchor contamination grouping threshold.
pub const DEFAULT_CONTAMINATION_THRESHOLD: f64 = 0.35;

/// Run the originality and contamination audit.
pub fn run_audit(options: &AuditOptions) -> anyhow::Result<AuditReport> {
    options.validate()?;
    let root = options
        .root
        .canonicalize()
        .with_context(|| format!("root {} does not exist", options.root.display()))?;
    let records = load_audit_records(&root, &options.records)?;
    let (references, reference_hash) = load_references(&options.references)?;
    let (splits, splits_hash) = load_splits(options.splits.as_ref())?;
    let (dispositions, dispositions_hash) = load_dispositions(options.dispositions.as_ref())?;
    if options.splits.is_some() {
        let missing: Vec<String> = records
            .iter()
            .map(|record| record.record.record_id())
            .filter(|id| !splits.contains_key(id))
            .collect();
        if !missing.is_empty() {
            bail!(
                "split assignments are incomplete for audited records: {}",
                missing.join(", ")
            );
        }
    }

    let mut flags = external_overlap_flags(&records, &references, options.overlap_threshold);
    let internal = scan_internal(
        &records,
        options.dedup_threshold,
        options.contamination_threshold,
        &splits,
    );
    let internal_count = internal.flags.len();
    let contamination_groups = internal.groups;
    flags.extend(internal.flags);
    let split_flags = cross_split_flags(&contamination_groups, options.contamination_threshold);
    let split_count = split_flags.len();
    flags.extend(split_flags);

    let external_count = count_kind(&flags, "external-overlap");
    let (flags, unused, applied) = apply_dispositions(flags, &dispositions);
    let needs_review = flags
        .iter()
        .filter(|flag| flag.status == "needs-review")
        .count();
    let blocking_count = flags.iter().filter(|flag| flag.release_blocking).count();
    let mut blocking_records: BTreeSet<String> = BTreeSet::new();
    for flag in flags.iter().filter(|flag| flag.release_blocking) {
        for id in &flag.record_ids {
            blocking_records.insert(id.clone());
        }
    }

    let mut thresholds = BTreeMap::new();
    thresholds.insert("external_overlap".to_owned(), options.overlap_threshold);
    thresholds.insert("internal_dedup".to_owned(), options.dedup_threshold);
    thresholds.insert("contamination".to_owned(), options.contamination_threshold);

    Ok(AuditReport {
        report_version: 1,
        inputs: InputSummary {
            records: records.len(),
            reference_items: references.len(),
            record_corpus_sha256: corpus_hash(&root, &records),
            reference_corpus_sha256: reference_hash,
            splits_sha256: splits_hash,
            dispositions_sha256: dispositions_hash,
        },
        thresholds,
        counts: CountSummary {
            external_pairs_screened: records.len() as u64 * references.len() as u64,
            internal_pairs_screened: internal.pairs,
            external_overlap_flags: external_count,
            internal_dedup_flags: internal_count,
            split_violation_flags: split_count,
            contamination_groups: contamination_groups.len(),
            needs_review,
            dispositions_applied: applied,
            release_blocking_flags: blocking_count,
        },
        records: record_summaries(&root, &records),
        reference_items: reference_summaries(&references),
        flags,
        split_audit: SplitSummary {
            status: if options.splits.is_some() {
                "checked".to_owned()
            } else {
                "not-run-no-split-assignments".to_owned()
            },
            assignments: splits.len(),
            violations: split_count,
        },
        contamination_groups,
        unused_disposition_ids: unused,
        release_blocking_record_ids: blocking_records.into_iter().collect(),
    })
}

/// Serialize a report as stable pretty JSON with a trailing newline.
pub fn report_json(report: &AuditReport) -> anyhow::Result<String> {
    let mut text =
        serde_json::to_string_pretty(report).context("cannot serialize originality report")?;
    text.push('\n');
    Ok(text)
}
