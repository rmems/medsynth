//! The admission pipeline's orchestration: load the batch, check the
//! pins, evaluate every record through the stages, run the review sample,
//! then write admitted shards and the admission log.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;
use serde_json::Value;

use crate::{Format, Validators};

use super::batch::{Batch, DraftRecord};
use super::claims;
use super::config::RunConfig;
use super::eval::{Ctx, RecordEval, evaluate};
use super::fulfill::Fulfillment;
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
    let batch = Batch::load(&opts.batch_dir)?;
    let registry = SourceRegistry::load(&opts.sources_path)?;
    let validators = Validators::compile();
    let fulfillment = Fulfillment::new(&opts.config.fulfillment_dir).map_err(|e| {
        format!(
            "cannot prepare fulfillment dir {}: {e}",
            opts.config.fulfillment_dir.display()
        )
    })?;

    // Fail-closed pin checks need the batch's shape.
    let needs_solver = batch
        .records
        .iter()
        .any(|r| matches!(r.format, Format::Mcq | Format::CaseVignette));
    let needs_verifier = batch
        .records
        .iter()
        .any(|r| !claims::claim_blocks(r).is_empty() || claims::distractor_screen(r).is_some());
    opts.config
        .check_pins(needs_solver, needs_verifier, batch.records.is_empty())?;

    let admitted_ids = admitted_record_ids(&opts.data_dir, &batch.batch_id)?;

    let ctx = Ctx {
        batch: &batch,
        validators: &validators,
        registry: &registry,
        config: &opts.config,
        fulfillment: &fulfillment,
        admitted_ids: &admitted_ids,
    };

    // Stages 0–4 per record, in parallel; order inside one record is strict.
    let mut results: Vec<RecordEval> = batch
        .records
        .par_iter()
        .map(|record| evaluate(&ctx, record))
        .collect::<Result<_, _>>()?;

    // Stage 5: batch-level review sample over records that cleared 0–4.
    let review = run_review(&ctx, &mut results)?;

    // Emit admitted shards.
    let mut outcomes = Vec::with_capacity(results.len());
    let mut admitted: Vec<&DraftRecord> = Vec::new();
    for eval in &results {
        if eval.outcome == Outcome::Admitted
            && let Some(record) = batch
                .records
                .iter()
                .find(|r| r.record_id() == eval.outcome_record.record_id && r.path == eval.path)
        {
            admitted.push(record);
        }
        outcomes.push(eval.outcome_record.clone());
    }
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

// ---------------------------------------------------------------------------
// Outputs
// ---------------------------------------------------------------------------

/// Ids already admitted under `data/` — excluding shards named for this
/// batch id so a re-run is idempotent rather than self-conflicting.
fn admitted_record_ids(data_dir: &Path, batch_id: &str) -> Result<HashSet<String>, String> {
    let mut ids = HashSet::new();
    for format in Format::all() {
        let dir = data_dir.join(format.slug());
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue; // a missing format dir holds no records
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let own_shard = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == format!("{batch_id}.jsonl"));
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") || own_shard {
                continue;
            }
            let text = std::fs::read_to_string(&path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            for line in text.lines() {
                if let Ok(v) = serde_json::from_str::<Value>(line)
                    && let Some(id) = v["id"].as_str()
                {
                    ids.insert(id.to_owned());
                }
            }
        }
    }
    Ok(ids)
}

/// Write admitted records to `data/<format>/<batch_id>.jsonl`, one compact
/// JSON line per record sorted by id — the only writes `data/` accepts.
fn write_admitted(
    data_dir: &Path,
    batch_id: &str,
    admitted: &[&DraftRecord],
    config: &RunConfig,
) -> Result<Vec<String>, String> {
    let mut shards = Vec::new();
    for format in Format::all() {
        let mut records: Vec<&DraftRecord> = admitted
            .iter()
            .filter(|r| r.format == format)
            .copied()
            .collect();
        if records.is_empty() {
            continue;
        }
        records.sort_by_key(|r| r.record_id());
        let dir = data_dir.join(format.slug());
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let mut text = String::new();
        for record in records {
            let mut value = record.value.clone();
            value["verification_status"] = Value::String("admitted".to_owned());
            value["contract_version"] = Value::String(config.contract_version.clone());
            let line = serde_json::to_string(&value)
                .map_err(|e| format!("serialize {}: {e}", record.record_id()))?;
            text.push_str(&line);
            text.push('\n');
        }
        let path = dir.join(format!("{batch_id}.jsonl"));
        write_text(&path, &text)
            .map_err(|e| format!("cannot write shard {}: {e}", path.display()))?;
        shards.push(display(&path));
    }
    Ok(shards)
}

fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    write_text(path, &text)
}

/// Write via a per-process temp file + rename so a failed run never leaves
/// a truncated shard or log behind.
fn write_text(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = std::fs::write(&tmp, text).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

pub(super) fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
