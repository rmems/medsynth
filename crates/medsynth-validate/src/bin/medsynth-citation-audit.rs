//! Citation-support audit binary (#26).
//!
//! Exit status is nonzero for operational failure or any `unsupported` verdict:
//! unsupported citations are release-blocking. `unverifiable` verdicts are
//! reported exhaustively rather than hidden; receipt-level-only sources are
//! named in `unverifiable_by_design_sources`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::Parser;
use medsynth_validate::citation_audit::{AuditOptions, report_json, run_audit};

/// Audit whether cited source passages support the claims that cite them.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Repository/corpus root used for report-relative paths.
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// Record file or directory to audit.
    #[arg(long, default_value = "data/gold")]
    records: PathBuf,
    /// License-compatible passage access policy.
    #[arg(long, default_value = "docs/citation-access.json")]
    access: PathBuf,
    /// Working passage cache outside data/.
    #[arg(long, default_value = "target/citation-audit-cache")]
    cache_dir: PathBuf,
    /// Independent-adjudicator request/response directory.
    #[arg(long, default_value = "target/citation-audit-fulfillment")]
    fulfillment_dir: PathBuf,
    /// Optional independent gold review manifest.
    #[arg(long)]
    gold_review: Option<PathBuf>,
    /// Drafting run id adjudicators must not equal (required for non-gold adjudication).
    #[arg(long)]
    drafting_run_id: Option<String>,
    /// Use only pre-populated cache entries.
    #[arg(long)]
    offline: bool,
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
        let resolved = if path.is_absolute() {
            path.clone()
        } else {
            root.join(path)
        };
        // Canonicalize inputs that already exist so corpus-relative display
        // paths match the canonicalized root even when the caller uses a
        // shortened Windows temp path.
        resolved.canonicalize().unwrap_or(resolved)
    };
    let report = run_audit(&AuditOptions {
        root: root.clone(),
        records: resolve(&cli.records),
        access: resolve(&cli.access),
        cache_dir: resolve(&cli.cache_dir),
        fulfillment_dir: resolve(&cli.fulfillment_dir),
        gold_review: cli.gold_review.as_ref().map(resolve),
        drafting_run_id: cli.drafting_run_id,
        offline: cli.offline,
    })?;
    let text = report_json(&report)?;
    if let Some(parent) = cli.report.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(&cli.report, text)
        .with_context(|| format!("cannot write report {}", cli.report.display()))?;
    println!(
        "medsynth-citation-audit: {} anchors, {} supported, {} unsupported, {} unverifiable -> {}",
        report.counts.anchors,
        report.counts.supported,
        report.counts.unsupported,
        report.counts.unverifiable,
        cli.report.display()
    );
    if report.unsupported() > 0 {
        bail!(
            "{} unsupported citation(s); see {}",
            report.unsupported(),
            cli.report.display()
        );
    }
    Ok(())
}
