//! The data-validation gate binary — runs every check over the repo's
//! record corpus and exits nonzero if any record fails.

use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::Parser;
use medsynth_validate::{Validators, load_corpus, run_checks};

/// Validate every record under `data/` and every `valid-*.json` fixture
/// under `fixtures/` against the schemas and rules in `medsynth-schema`.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Repository root to validate.
    #[arg(default_value = ".")]
    root: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let root = cli
        .root
        .canonicalize()
        .with_context(|| format!("root {} does not exist", cli.root.display()))?;

    let validators = Validators::compile();
    let (records, mut findings) = load_corpus(&root);
    findings.extend(run_checks(&records, &validators));

    for finding in &findings {
        eprintln!("{finding}");
    }
    if findings.is_empty() {
        println!(
            "medsynth-validate: {} record(s), all checks passed",
            records.len()
        );
        Ok(())
    } else {
        bail!(
            "{} check failure(s) across {} record(s)",
            findings.len(),
            records.len()
        );
    }
}
