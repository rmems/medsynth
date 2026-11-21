//! The data-validation gate binary — runs every check over the repo's
//! record corpus and exits nonzero if any record fails.

use std::path::PathBuf;

use anyhow::{Context, bail};
use clap::Parser;
use medsynth_validate::{Validators, load_corpus, load_registry, run_checks};

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
    let registry = load_registry(&root);
    let (records, mut findings) = load_corpus(&root);
    findings.extend(run_checks(&records, &validators, &registry));

    for finding in &findings {
        eprintln!("{finding}");
    }
    if !findings.is_empty() {
        bail!(
            "{} check failure(s) across {} record(s)",
            findings.len(),
            records.len()
        );
    }
    // An empty corpus passing silently would let a broken checkout or a
    // moved data/ directory green-light the gate — fail loudly instead.
    if records.is_empty() {
        bail!(
            "no records found under {}/data or {}/fixtures",
            root.display(),
            root.display()
        );
    }
    println!(
        "medsynth-validate: {} record(s), all checks passed",
        records.len()
    );
    Ok(())
}
