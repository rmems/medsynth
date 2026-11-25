//! The admission pipeline's orchestration: load the batch, check the
//! pins, evaluate every record through the stages, run the review sample,
//! then write admitted shards and the admission log.

use std::collections::{BTreeMap, HashMap};

use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;

use crate::Format;

use super::batch::{Batch, DraftRecord};
use super::claims;
use super::config::RunConfig;
use super::eval::{Ctx, RecordEval, evaluate};
use super::fulfill::Fulfillment;
use super::outputs::{display, write_admitted, write_json};
use super::published::published_record_ids;
use super::registry::SourceRegistry;
use super::review::run_review;

/// A pipeline stage, in execution order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Stage {
    /// Rights-envelope and intake checks before the stages proper.
    Intake = 0,
    Schema = 1,
    Key = 2,
    Anchor = 3,
    Distractor = 4,
    Review = 5,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Self::Intake => "intake",
            Self::Schema => "schema",
            Self::Key => "key",
            Self::Anchor => "anchor",
            Self::Distractor => "distractor",
            Self::Review => "review",
        }
    }
}

impl From<Stage> for u8 {
    fn from(stage: Stage) -> u8 {
        stage as u8
    }
}

/// How a record left the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Admitted,
    Rejected,
    Held,
}

/// The report entry for one record.
#[derive(Debug, Clone, Serialize)]
pub struct RecordOutcome {
    pub record_id: String,
    pub format: String,
    pub path: String,
    pub outcome: Outcome,
    /// The stage that rejected or held the record (absent when admitted).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_name: Option<String>,
    /// The highest verification status the record earned this run —
    /// `unverified` until stage 2 passes.
    pub status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
}

/// Which pins and parameters this run operated under.
#[derive(Debug, Serialize)]
pub struct RunReport {
    pub solver_run_id: Option<String>,
    pub solver_model_id: Option<String>,
    pub verifier_run_id: Option<String>,
    pub reviewer_run_id: Option<String>,
    pub sample_rate: f64,
    pub contract_version: String,
    /// The `docs/sources.json` version anchors were checked against.
    pub registry_version: String,
    pub fulfillment_dir: String,
}

/// The stage-5 review section of the log.
#[derive(Debug, Serialize)]
pub struct ReviewReport {
    pub sample_rate: f64,
    pub sampled: Vec<String>,
    pub results: Vec<ReviewResult>,
}

#[derive(Debug, Serialize)]
pub struct ReviewResult {
    pub record_id: String,
    pub reviewer_run_id: String,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Aggregate counts.
#[derive(Debug, Default, Serialize)]
pub struct Counts {
    pub input: usize,
    pub admitted: usize,
    pub rejected: usize,
    pub held: usize,
}

/// The per-batch admission log (contract §8: the gate's bite is measured
/// here).
#[derive(Debug, Serialize)]
pub struct AdmissionReport {
    pub log_version: u32,
    pub batch_id: String,
    pub batch_dir: String,
    pub run: RunReport,
    pub counts: Counts,
    /// Rejections per stage — `{"2-key": 2}` — for the contract's
    /// rejected-per-stage accounting.
    pub rejected_by_stage: BTreeMap<String, u32>,
    /// Holds per stage, reported separately from rejections.
    pub held_by_stage: BTreeMap<String, u32>,
    pub outcomes: Vec<RecordOutcome>,
    pub review: ReviewReport,
    /// Data shards this run wrote.
    pub admitted_shards: Vec<String>,
}

/// Inputs to [`run`].
pub struct AdmissionOptions {
    /// Draft batch directory (manifest optional).
    pub batch_dir: PathBuf,
    /// The admitted dataset tree — `data/`.
    pub data_dir: PathBuf,
    /// `docs/sources.json`.
    pub sources_path: PathBuf,
    pub config: RunConfig,
    /// Where the admission log JSON is written.
    pub log_path: PathBuf,
}

/// Run the admission pipeline and write its outputs. Returns the report
/// that was also written to `log_path`.
pub fn run(opts: &AdmissionOptions) -> Result<AdmissionReport, String> {
    // Validate regardless of how the config was constructed — library
    // callers build `RunConfig` directly, bypassing `load_run_config`.
    opts.config.validate()?;
    let batch = Batch::load(&opts.batch_dir)?;
    let registry = SourceRegistry::load(&opts.sources_path)?;
    let validators = crate::Validators::compile();
    let fulfillment = Fulfillment::new(&opts.config.fulfillment_dir).map_err(|e| {
        format!(
            "cannot prepare fulfillment dir {}: {e}",
            opts.config.fulfillment_dir.display()
        )
    })?;

    check_batch_pins(&batch, &opts.config)?;
    check_log_path(&opts.data_dir, &batch.batch_id, &opts.log_path)?;
    let admitted_ids = published_record_ids(&opts.data_dir, &batch.batch_id)?;

    // Stage 1's duplicate check reads counts, not the whole batch.
    let mut id_counts: HashMap<String, usize> = HashMap::new();
    for record in &batch.records {
        *id_counts.entry(record.record_id()).or_default() += 1;
    }

    let ctx = Ctx {
        batch: &batch,
        validators: &validators,
        registry: &registry,
        config: &opts.config,
        fulfillment: &fulfillment,
        admitted_ids: &admitted_ids,
        id_counts: &id_counts,
    };

    // Stages 0–4 per record, in parallel; order inside one record is strict.
    let mut results: Vec<RecordEval> = batch
        .records
        .par_iter()
        .map(|record| evaluate(&ctx, record))
        .collect::<Result<_, _>>()?;

    // Stage 5: batch-level review sample over records that cleared 0–4.
    let review = run_review(&ctx, &mut results)?;

    let (outcomes, admitted) = collect_outcomes(&results, &batch);
    let admitted_shards = write_admitted(&opts.data_dir, &batch.batch_id, &admitted, &opts.config)?;

    let report = AdmissionReport {
        log_version: 1,
        batch_id: batch.batch_id.clone(),
        batch_dir: display(&opts.batch_dir),
        run: RunReport {
            solver_run_id: opts.config.solver.as_ref().map(|s| s.run_id.clone()),
            solver_model_id: opts.config.solver.as_ref().map(|s| s.model_id.clone()),
            verifier_run_id: opts.config.verifier.as_ref().map(|a| a.run_id.clone()),
            reviewer_run_id: opts.config.reviewer.as_ref().map(|a| a.run_id.clone()),
            sample_rate: opts.config.sample_rate,
            contract_version: opts.config.contract_version.clone(),
            registry_version: registry.registry_version.clone(),
            fulfillment_dir: display(&opts.config.fulfillment_dir),
        },
        counts: count(&results),
        rejected_by_stage: per_stage(&results, Outcome::Rejected),
        held_by_stage: per_stage(&results, Outcome::Held),
        outcomes,
        review,
        admitted_shards,
    };
    write_json(&opts.log_path, &report).map_err(|e| {
        format!(
            "cannot write admission log {}: {e}",
            opts.log_path.display()
        )
    })?;
    Ok(report)
}

/// The log is written after the shards, so a log path colliding with a
/// shard destination would silently replace the shard — refuse to start.
fn check_log_path(data_dir: &Path, batch_id: &str, log_path: &Path) -> Result<(), String> {
    let absolute_log = std::path::absolute(log_path).unwrap_or_else(|_| log_path.to_path_buf());
    for format in Format::all() {
        let shard = data_dir
            .join(format.slug())
            .join(format!("{batch_id}.jsonl"));
        let absolute_shard = std::path::absolute(&shard).unwrap_or(shard);
        if absolute_log == absolute_shard {
            return Err(format!(
                "admission log path {} collides with the {} shard destination",
                log_path.display(),
                format.slug()
            ));
        }
    }
    Ok(())
}

/// Fail-closed pin checks need the batch's shape — which semantic stages
/// will actually fire.
fn check_batch_pins(batch: &Batch, config: &RunConfig) -> Result<(), String> {
    let needs_solver = batch
        .records
        .iter()
        .any(|r| matches!(r.format, Format::Mcq | Format::CaseVignette));
    let needs_verifier = batch
        .records
        .iter()
        .any(|r| !claims::claim_blocks(r).is_empty() || claims::distractor_screen(r).is_some());
    config.check_pins(needs_solver, needs_verifier, batch.records.is_empty())
}

/// The log's per-record outcomes plus the records to write to shards.
fn collect_outcomes<'a>(
    results: &[RecordEval],
    batch: &'a Batch,
) -> (Vec<RecordOutcome>, Vec<&'a DraftRecord>) {
    let mut outcomes = Vec::with_capacity(results.len());
    let mut admitted = Vec::new();
    for eval in results {
        if eval.outcome == Outcome::Admitted {
            admitted.extend(
                batch.records.iter().find(|r| {
                    r.record_id() == eval.outcome_record.record_id && r.path == eval.path
                }),
            );
        }
        outcomes.push(eval.outcome_record.clone());
    }
    (outcomes, admitted)
}

fn count(results: &[RecordEval]) -> Counts {
    let mut counts = Counts {
        input: results.len(),
        ..Counts::default()
    };
    for eval in results {
        match eval.outcome {
            Outcome::Admitted => counts.admitted += 1,
            Outcome::Rejected => counts.rejected += 1,
            Outcome::Held => counts.held += 1,
        }
    }
    counts
}

/// `{"2-key": 2}`-style per-stage tallies for one outcome kind.
fn per_stage(results: &[RecordEval], outcome: Outcome) -> BTreeMap<String, u32> {
    let mut tally = BTreeMap::new();
    for eval in results.iter().filter(|e| e.outcome == outcome) {
        *tally.entry(eval.stage_label()).or_insert(0u32) += 1;
    }
    tally
}
