//! The admission gate binary — runs the verification pipeline over a draft
//! batch, writes admitted records to `data/<format>/`, and emits the
//! per-batch admission log.
//!
//! Exit status: `0` means the gate ran to completion and wrote its log —
//! the log is the verdict (rejections and holds live there, not in the
//! exit code). `1` means an operational failure: unreadable batch or
//! registry, missing pins the batch requires, or I/O errors.

use std::path::PathBuf;

use anyhow::bail;
use clap::Parser;
use medsynth_validate::admit::{self, AdmissionOptions};

/// Gate a draft batch through the verification pipeline: intake rights
/// envelope, schema, answer-key re-derivation, source anchors, distractor
/// plausibility, and the agent review sample.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Draft batch directory (draft .jsonl files plus the batch manifest).
    #[arg(long)]
    batch: PathBuf,
    /// Run config JSON: solver/verifier/reviewer pins, sample rate,
    /// contract version, fulfillment directory.
    #[arg(long)]
    config: PathBuf,
    /// Source registry the anchors are checked against.
    #[arg(long, default_value = "docs/sources.json")]
    sources: PathBuf,
    /// Dataset root admitted records are written under (`<format>/<batch>.jsonl`).
    #[arg(long, default_value = "data")]
    data_dir: PathBuf,
    /// Where the per-batch admission log JSON is written.
    #[arg(long)]
    log: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config = admit::load_run_config(&cli.config)
        .map_err(|e| anyhow::anyhow!("cannot load run config {}: {e}", cli.config.display()))?;
    let report = admit::run(&AdmissionOptions {
        batch_dir: cli.batch.clone(),
        sources_path: cli.sources.clone(),
        data_dir: cli.data_dir.clone(),
        log_path: cli.log.clone(),
        config,
    })
    .map_err(anyhow::Error::msg)?;

    println!(
        "medsynth-admit {}: {} in, {} admitted, {} rejected, {} held -> {}",
        report.batch_id,
        report.counts.input,
        report.counts.admitted,
        report.counts.rejected,
        report.counts.held,
        cli.log.display(),
    );
    if !report.admitted_shards.is_empty() {
        println!("  shards: {}", report.admitted_shards.join(", "));
    }
    if report.counts.input == 0 {
        bail!("batch {} held no records", cli.batch.display());
    }
    Ok(())
}
