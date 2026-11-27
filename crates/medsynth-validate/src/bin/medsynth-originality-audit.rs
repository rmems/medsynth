//! Originality and contamination audit binary (#11).
//!
//! Exit status is nonzero for operational failure or any release-blocking
//! unresolved/confirmed overlap flag. The report is deterministic: it carries
//! hashes, ids, metrics, and dispositions, never external reference text.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::Parser;
use medsynth_validate::originality_audit::{
    AuditOptions, DEFAULT_CONTAMINATION_THRESHOLD, DEFAULT_DEDUP_THRESHOLD,
    DEFAULT_OVERLAP_THRESHOLD, report_json, run_audit,
};

/// Audit overlap, internal duplication, and split-aware contamination risk.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Repository/corpus root used for report-relative paths.
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// Record file or directory to audit; repeat for multiple inputs.
    #[arg(long, default_value = "data/gold")]
    records: Vec<PathBuf>,
    /// JSONL reference corpus of public sample items.
    #[arg(long, default_value = "docs/contamination-reference.jsonl")]
    references: PathBuf,
    /// Optional split assignments JSON.
    #[arg(long)]
    splits: Option<PathBuf>,
    /// Optional human-review dispositions JSON.
    #[arg(long)]
    dispositions: Option<PathBuf>,
    /// External overlap review threshold.
    #[arg(long, default_value_t = DEFAULT_OVERLAP_THRESHOLD)]
    overlap_threshold: f64,
    /// Internal same-cell duplicate review threshold.
    #[arg(long, default_value_t = DEFAULT_DEDUP_THRESHOLD)]
    dedup_threshold: f64,
    /// Same-passage contamination grouping threshold.
    #[arg(long, default_value_t = DEFAULT_CONTAMINATION_THRESHOLD)]
    contamination_threshold: f64,
    /// Where the audit report JSON is written.
    #[arg(long)]
    report: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let root = cli
        .root
        .canonicalize()
        .with_context(|| format!("root {} does not exist", cli.root.display()))?;
    let resolve = |path: &PathBuf| {
        if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        }
    };
    let report = run_audit(&AuditOptions {
        root: root.clone(),
        records: cli.records.iter().map(resolve).collect(),
        references: resolve(&cli.references),
        splits: cli.splits.as_ref().map(resolve),
        dispositions: cli.dispositions.as_ref().map(resolve),
        overlap_threshold: cli.overlap_threshold,
        dedup_threshold: cli.dedup_threshold,
        contamination_threshold: cli.contamination_threshold,
    })?;
    let report_path = resolve(&cli.report);
    if let Some(parent) = report_path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(&report_path, report_json(&report)?)
        .with_context(|| format!("cannot write report {}", report_path.display()))?;
    println!(
        "medsynth-originality-audit: {} records, {} reference items, {} flags ({} release-blocking) -> {}",
        report.inputs.records,
        report.inputs.reference_items,
        report.flags.len(),
        report.counts.release_blocking_flags,
        report_path.display()
    );
    if report.release_blocking() {
        bail!(
            "{} release-blocking originality flag(s); see {}",
            report.counts.release_blocking_flags,
            report_path.display()
        );
    }
    Ok(())
}
