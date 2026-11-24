//! The admission pipeline: intake plus five ordered stages per record, then
//! the batch-level review sample.
//!
//! Stage verdicts are `Reject` (a genuine defect — the record never
//! reaches `data/`) or `Hold` (something that must be resolved before the
//! record can be judged — a stale registry hash, a missing fulfillment,
//! a failed review). Holds are reported separately from rejections so
//! infrastructure and provenance issues never masquerade as defects
//! (contract §8).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use serde::Serialize;
use serde_json::Value;

use crate::{Format, RecordFile, Validators, run_schema_check};

use super::batch::{Batch, DraftRecord};
use super::claims;
use super::config::RunConfig;
use super::fulfill::{Exchange, Fulfillment, RequestKind};
use super::registry::SourceRegistry;

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

/// One stage's verdict on a record.
enum Verdict {
    Pass,
    Reject(Vec<String>),
    Hold(Vec<String>),
}

impl Verdict {
    fn reject(reason: impl Into<String>) -> Self {
        Self::Reject(vec![reason.into()])
    }
    fn hold(reason: impl Into<String>) -> Self {
        Self::Hold(vec![reason.into()])
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

/// Everything a record evaluation reads.
struct Ctx<'a> {
    batch: &'a Batch,
    validators: &'a Validators,
    registry: &'a SourceRegistry,
    config: &'a RunConfig,
    fulfillment: &'a Fulfillment,
    /// Ids already admitted under `data/` (minus this run's own shards) —
    /// a re-admission is a stage-1 duplicate.
    admitted_ids: &'a HashSet<String>,
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

    let mut counts = Counts {
        input: results.len(),
        ..Counts::default()
    };
    let mut rejected_by_stage: BTreeMap<String, u32> = BTreeMap::new();
    let mut held_by_stage: BTreeMap<String, u32> = BTreeMap::new();
    for eval in &results {
        match eval.outcome {
            Outcome::Admitted => counts.admitted += 1,
            Outcome::Rejected => {
                counts.rejected += 1;
                *rejected_by_stage.entry(eval.stage_label()).or_default() += 1;
            }
            Outcome::Held => {
                counts.held += 1;
                *held_by_stage.entry(eval.stage_label()).or_default() += 1;
            }
        }
    }

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
        counts,
        rejected_by_stage,
        held_by_stage,
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

// ---------------------------------------------------------------------------
// Per-record evaluation
// ---------------------------------------------------------------------------

struct RecordEval {
    outcome: Outcome,
    outcome_record: RecordOutcome,
    path: PathBuf,
    stage: Option<Stage>,
    /// Records that cleared stages 0–4 and entered the review pool.
    review_candidate: bool,
}

impl RecordEval {
    fn stage_label(&self) -> String {
        self.stage
            .map(|s| format!("{}-{}", u8::from(s), s.name()))
            .unwrap_or_else(|| "admitted".to_owned())
    }
}

/// Wrap a non-pass verdict into a [`RecordEval`] — `status` is the highest
/// stage the record already cleared.
fn stopped(record: &DraftRecord, verdict: Verdict, stage: Stage, status: &str) -> RecordEval {
    let (outcome, reasons) = match verdict {
        Verdict::Pass => unreachable!("stopped() only sees rejects and holds"),
        Verdict::Reject(r) => (Outcome::Rejected, r),
        Verdict::Hold(r) => (Outcome::Held, r),
    };
    RecordEval {
        outcome,
        outcome_record: RecordOutcome {
            record_id: record.record_id(),
            format: record.format.slug().to_owned(),
            path: display(&record.path),
            outcome,
            stage: Some(stage.into()),
            stage_name: Some(stage.name().to_owned()),
            status: status.to_owned(),
            reasons,
        },
        path: record.path.clone(),
        stage: Some(stage),
        review_candidate: false,
    }
}

fn evaluate(ctx: &Ctx, record: &DraftRecord) -> Result<RecordEval, String> {
    let mut status = "unverified";
    for (stage, v) in [
        (Stage::Intake, stage_intake(ctx, record)),
        (Stage::Schema, stage_schema(ctx, record)),
    ] {
        if !matches!(v, Verdict::Pass) {
            return Ok(stopped(record, v, stage, status));
        }
    }
    let v = stage_key(ctx, record)?;
    if !matches!(v, Verdict::Pass) {
        return Ok(stopped(record, v, Stage::Key, status));
    }
    status = "key-checked";
    let (v, verify_response) = stage_anchor(ctx, record)?;
    if !matches!(v, Verdict::Pass) {
        return Ok(stopped(record, v, Stage::Anchor, status));
    }
    status = "anchor-checked";
    let v = stage_distractor(record, verify_response.as_ref());
    if !matches!(v, Verdict::Pass) {
        return Ok(stopped(record, v, Stage::Distractor, status));
    }
    // Cleared stages 0–4; the review pool decides admitted vs held.
    Ok(RecordEval {
        outcome: Outcome::Admitted,
        outcome_record: RecordOutcome {
            record_id: record.record_id(),
            format: record.format.slug().to_owned(),
            path: display(&record.path),
            outcome: Outcome::Admitted,
            stage: None,
            stage_name: None,
            status: status.to_owned(),
            reasons: Vec::new(),
        },
        path: record.path.clone(),
        stage: None,
        review_candidate: true,
    })
}

// ---------------------------------------------------------------------------
// Stage 0 — intake / rights envelope
// ---------------------------------------------------------------------------

fn stage_intake(ctx: &Ctx, record: &DraftRecord) -> Verdict {
    if record.value["verification_status"].as_str() != Some("unverified") {
        return Verdict::reject(format!(
            "verification_status is `{}`, not `unverified` — only drafts enter admission",
            record.value["verification_status"]
                .as_str()
                .unwrap_or("<missing>")
        ));
    }
    let id = record.record_id();
    let entry = ctx.batch.record_entry(&id);
    if ctx.batch.has_manifest() && entry.is_none() {
        return Verdict::reject("record is not listed in the batch manifest");
    }
    if let Some(entry) = entry
        && let Some(manifest_sha) = entry["sha256"].as_str()
        && manifest_sha != record.content_sha256()
    {
        return Verdict::reject("record content diverges from the manifest's sha256");
    }
    match lane_of(record, entry).as_str() {
        "teacher" => intake_teacher(ctx, record),
        "procedural" => intake_procedural(record, entry),
        _ => Verdict::Pass,
    }
}

/// The drafting lane: the manifest's `records[].lane` when present, else
/// inferred from `provenance` — `template:` generators are procedural, a
/// recorded `model` makes the draft teacher-lane, anything else is
/// agent-drafted and carries no envelope.
fn lane_of(record: &DraftRecord, entry: Option<&Value>) -> String {
    if let Some(lane) = entry.and_then(|e| e["lane"].as_str()) {
        return lane.to_owned();
    }
    let provenance = &record.value["provenance"];
    if provenance["generator"]
        .as_str()
        .is_some_and(|g| g.starts_with("template:"))
    {
        return "procedural".to_owned();
    }
    if provenance["model"].as_str().is_some_and(|m| !m.is_empty()) {
        return "teacher".to_owned();
    }
    "agent".to_owned()
}

fn intake_teacher(ctx: &Ctx, record: &DraftRecord) -> Verdict {
    let id = record.record_id();
    let Some(envelope) = ctx.batch.rights_envelope(&id) else {
        return Verdict::reject("teacher-lane draft has no rights envelope in the manifest");
    };
    let mut missing = Vec::new();
    for field in [
        "record_id",
        "teacher_variant",
        "model_id",
        "pin_id",
        "pin_entry",
        "pin_entry_sha256",
        "content_sha256",
    ] {
        if envelope[field].as_str().is_none_or(|s| s.trim().is_empty()) {
            missing.push(field);
        }
    }
    if !missing.is_empty() {
        return Verdict::reject(format!(
            "incomplete rights envelope — missing: {}",
            missing.join(", ")
        ));
    }
    if envelope["record_id"].as_str() != Some(id.as_str()) {
        return Verdict::reject("rights envelope names a different record id");
    }
    if envelope["content_sha256"].as_str() != Some(record.content_sha256().as_str()) {
        return Verdict::reject("rights envelope content_sha256 does not match the record");
    }
    Verdict::Pass
}

fn intake_procedural(record: &DraftRecord, entry: Option<&Value>) -> Verdict {
    let provenance = &record.value["provenance"];
    if !provenance["generator"]
        .as_str()
        .is_some_and(|g| g.starts_with("template:"))
        || provenance["template_version"]
            .as_str()
            .is_none_or(|t| t.is_empty())
    {
        return Verdict::reject(
            "procedural-lane draft lacks its `template:` generator or template_version",
        );
    }
    if let Some(entry) = entry
        && (!entry["seed"].is_u64() || entry["generator"].as_str().is_none_or(|g| g.is_empty()))
    {
        return Verdict::reject(
            "procedural-lane manifest entry lacks its seed or template generator",
        );
    }
    Verdict::Pass
}

// ---------------------------------------------------------------------------
// Stage 1 — schema
// ---------------------------------------------------------------------------

fn stage_schema(ctx: &Ctx, record: &DraftRecord) -> Verdict {
    let mut reasons: Vec<String> = Vec::new();
    let record_file = RecordFile {
        path: record.path.clone(),
        line: record.line,
        format: record.format,
        value: record.value.clone(),
        raw: record.raw.clone(),
    };
    for finding in run_schema_check(&record_file, ctx.validators) {
        reasons.push(finding.detail);
    }
    let id = record.record_id();
    if ctx
        .batch
        .records
        .iter()
        .filter(|r| r.record_id() == id)
        .count()
        > 1
    {
        reasons.push("duplicate record id within the batch".to_owned());
    }
    if ctx.admitted_ids.contains(&id) {
        reasons.push("record id is already admitted under data/".to_owned());
    }
    if reasons.is_empty() {
        Verdict::Pass
    } else {
        Verdict::Reject(reasons)
    }
}

// ---------------------------------------------------------------------------
// Stage 2 — answer-key re-derivation
// ---------------------------------------------------------------------------

fn stage_key(ctx: &Ctx, record: &DraftRecord) -> Result<Verdict, String> {
    let Some(item) = claims::blind_item(record) else {
        return Ok(Verdict::Pass); // tutoring_example: no key to re-derive
    };
    let solver = ctx
        .config
        .solver
        .as_ref()
        .expect("check_pins ran before evaluate");

    // The solver must never be the model/run that drafted the record.
    // Check the rights envelope first, then provenance — before emitting a
    // solve request at all.
    if same_model(ctx, record, &solver.model_id, solver.pin_id.as_deref()) {
        return Ok(Verdict::reject(
            "stage-2 solver is the model/run that drafted this record",
        ));
    }

    let body = claims::solve_body(&ctx.batch.batch_id, record, &item);
    let value = match ctx.fulfillment.exchange(RequestKind::Solve, &body) {
        Ok(Exchange::Response { value, .. }) => value,
        Ok(Exchange::Unavailable { key }) => {
            return Ok(Verdict::hold(format!(
                "solver fulfillment unavailable ({key})"
            )));
        }
        Ok(Exchange::Malformed { key, detail }) => {
            return Ok(Verdict::hold(format!(
                "solver response {key} is not usable JSON: {detail}"
            )));
        }
        Err(e) => return Err(format!("solver fulfillment I/O: {e}")),
    };
    if let Some(verdict) = identity_ok(&value, &solver.run_id, Some(&solver.model_id), "solver") {
        return Ok(verdict);
    }
    Ok(match record.format {
        Format::Mcq => {
            let key = record.value["key"].as_str().unwrap_or("");
            match value["answer"].as_str() {
                Some(answer) if answer == key => Verdict::Pass,
                Some(answer) => Verdict::reject(format!(
                    "solver derived `{answer}` but the record keys `{key}`"
                )),
                None => Verdict::hold("solver response carries no `answer`"),
            }
        }
        Format::CaseVignette => match value["verdict"].as_str() {
            Some("agree") => Verdict::Pass,
            Some("disagree") => Verdict::reject(format!(
                "solver disagrees with the expected answer{}",
                detail_suffix(&value)
            )),
            _ => Verdict::hold("solver response carries no `verdict`"),
        },
        Format::TutoringExample => Verdict::Pass,
    })
}

/// Whether `model_id`/`pin_id` collides with the drafting identity: the
/// rights envelope's model, served model, or pin, or `provenance.model`.
fn same_model(ctx: &Ctx, record: &DraftRecord, model_id: &str, pin_id: Option<&str>) -> bool {
    let id = record.record_id();
    if let Some(envelope) = ctx.batch.rights_envelope(&id) {
        let model_hit = ["model_id", "served_model", "teacher_variant"]
            .iter()
            .any(|f| envelope[f].as_str() == Some(model_id));
        let pin_hit = pin_id.is_some() && envelope["pin_id"].as_str() == pin_id;
        if model_hit || pin_hit {
            return true;
        }
    }
    record.value["provenance"]["model"].as_str() == Some(model_id)
}

/// Verify a fulfillment response names the pinned run (and model, when
/// pinned). A mismatch is a hold — the harness answered a different request
/// than the gate sent.
fn identity_ok(
    response: &Value,
    run_id: &str,
    model_id: Option<&str>,
    label: &str,
) -> Option<Verdict> {
    if response["run_id"].as_str() != Some(run_id) {
        return Some(Verdict::hold(format!(
            "{label} response names run `{}`, not the pinned `{run_id}`",
            response["run_id"].as_str().unwrap_or("<missing>")
        )));
    }
    if let Some(model) = model_id
        && let Some(served) = response["model_id"].as_str()
        && served != model
    {
        return Some(Verdict::hold(format!(
            "{label} response names model `{served}`, not the pinned `{model}`"
        )));
    }
    None
}

fn detail_suffix(response: &Value) -> String {
    response["detail"]
        .as_str()
        .map(|d| format!(": {d}"))
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Stage 3 — source anchors and claim support
// ---------------------------------------------------------------------------

fn stage_anchor(ctx: &Ctx, record: &DraftRecord) -> Result<(Verdict, Option<Value>), String> {
    let claim_blocks = claims::claim_blocks(record);
    let mut rejections = Vec::new();
    let mut holds = Vec::new();

    // Deterministic resolution: registry membership, currency, locator
    // syntax, and #8's anchor receipts against the current registry.
    let mut locator_validators: HashMap<String, jsonschema::Validator> = HashMap::new();
    for block in &claim_blocks {
        for (source_id, locator) in &block.anchors {
            let Some(entry) = ctx.registry.get(source_id) else {
                rejections.push(format!(
                    "{}: cites `{source_id}`, which is not in the source registry",
                    block.path
                ));
                continue;
            };
            if !entry.is_current() {
                holds.push(format!(
                    "{}: `{source_id}` is `{}` — held for re-anchoring",
                    block.path, entry.status
                ));
            }
            let validator = match locator_validators.entry(source_id.clone()) {
                std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
                std::collections::hash_map::Entry::Vacant(e) => {
                    let pattern = medsynth_schema::locator::pattern(&entry.locator_format)
                        .ok_or_else(|| {
                            format!(
                                "registry entry `{source_id}` has an unrecognized \
                                 locator convention `{}`",
                                entry.locator_format
                            )
                        })?;
                    e.insert(
                        jsonschema::validator_for(&serde_json::json!({
                            "type": "string",
                            "pattern": pattern,
                        }))
                        .expect("locator patterns compile"),
                    )
                }
            };
            if !validator.is_valid(&serde_json::json!(locator)) {
                rejections.push(format!(
                    "{}: locator `{locator}` violates `{source_id}`'s convention `{}`",
                    block.path, entry.locator_format
                ));
            }
            // #8's anchor receipts pin the registry entry bytes at draft
            // time; a drifted entry holds the record for re-anchoring.
            if ctx.batch.has_anchor_receipts() {
                match ctx
                    .batch
                    .anchor_receipt(&record.record_id(), source_id, locator)
                {
                    None => holds.push(format!(
                        "{path}: anchor {source_id} {locator} has no batch receipt",
                        path = block.path
                    )),
                    Some(receipt)
                        if receipt["registry_entry_sha256"].as_str()
                            != Some(entry.sha256.as_str()) =>
                    {
                        holds.push(format!(
                            "{}: registry entry `{source_id}` changed since drafting — \
                             held for re-anchoring",
                            block.path
                        ));
                    }
                    Some(_) => {}
                }
            }
        }
    }

    // Semantic support: every claim block gets a verdict from the pinned
    // verifier — supported, claim-free, unsupported, or unanchored. A
    // record with neither claims nor a distractor screen has nothing
    // semantic to check; deterministic results alone decide.
    let distractor = claims::distractor_screen(record);
    if claim_blocks.is_empty() && distractor.is_none() {
        return Ok((
            if !rejections.is_empty() {
                Verdict::Reject(rejections)
            } else if !holds.is_empty() {
                Verdict::Hold(holds)
            } else {
                Verdict::Pass
            },
            None,
        ));
    }
    let body = claims::verify_body(
        &ctx.batch.batch_id,
        record,
        &claim_blocks,
        distractor.as_ref(),
    );
    let response = match ctx.fulfillment.exchange(RequestKind::Verify, &body) {
        Ok(Exchange::Response { value, .. }) => Some(value),
        Ok(Exchange::Unavailable { key }) => {
            holds.push(format!("verifier fulfillment unavailable ({key})"));
            None
        }
        Ok(Exchange::Malformed { key, detail }) => {
            holds.push(format!(
                "verifier response {key} is not usable JSON: {detail}"
            ));
            None
        }
        Err(e) => return Err(format!("verifier fulfillment I/O: {e}")),
    };
    let verifier_run_id = &ctx.config.verifier.as_ref().expect("check_pins ran").run_id;
    if let Some(value) = &response {
        if let Some(verdict) = identity_ok(value, verifier_run_id, None, "verifier") {
            holds.extend(match verdict {
                Verdict::Hold(r) | Verdict::Reject(r) => r,
                Verdict::Pass => Vec::new(),
            });
        }
        let verdicts: HashMap<&str, &Value> = value["claims"]
            .as_array()
            .map(|vs| {
                vs.iter()
                    .filter_map(|v| v["path"].as_str().map(|p| (p, v)))
                    .collect()
            })
            .unwrap_or_default();
        for block in &claim_blocks {
            match verdicts.get(block.path.as_str()) {
                None => holds.push(format!("verifier returned no verdict for {}", block.path)),
                Some(v) => match v["verdict"].as_str() {
                    Some("supported") | Some("no-claim") => {}
                    Some("unanchored") => rejections.push(format!(
                        "{}: unanchored factual claim{}",
                        block.path,
                        detail_suffix(v)
                    )),
                    Some("unsupported") => rejections.push(format!(
                        "{}: claim not supported by the cited passage{}",
                        block.path,
                        detail_suffix(v)
                    )),
                    other => holds.push(format!(
                        "verifier returned unknown verdict `{}` for {}",
                        other.unwrap_or("<missing>"),
                        block.path
                    )),
                },
            }
        }
    }

    let verdict = if !rejections.is_empty() {
        Verdict::Reject(rejections)
    } else if !holds.is_empty() {
        Verdict::Hold(holds)
    } else {
        Verdict::Pass
    };
    Ok((verdict, response))
}

// ---------------------------------------------------------------------------
// Stage 4 — distractor plausibility
// ---------------------------------------------------------------------------

fn stage_distractor(record: &DraftRecord, verify_response: Option<&Value>) -> Verdict {
    if record.format != Format::Mcq {
        return Verdict::Pass;
    }
    // The statable-reason requirement: a blank refutation never survives
    // stage 1, but the gate re-checks rather than trusting the schema.
    if record.value["rationale"]["strongest_distractor"]["text"]
        .as_str()
        .is_none_or(|t| t.trim().is_empty())
    {
        return Verdict::reject("rationale records no reason the strongest distractor is wrong");
    }
    let Some(response) = verify_response else {
        return Verdict::hold("no verifier response for the distractor screen");
    };
    match response["distractor"]["verdict"].as_str() {
        Some("pass") => Verdict::Pass,
        Some("fail") => Verdict::reject(format!(
            "distractor screen{}",
            detail_suffix(&response["distractor"])
        )),
        _ => Verdict::hold("verifier returned no distractor verdict"),
    }
}

// ---------------------------------------------------------------------------
// Stage 5 — accuracy review sample
// ---------------------------------------------------------------------------

/// Deterministic sample of `n` candidates at `rate` — never fewer than
/// `min(5, n)`. Evenly spaced over the id-sorted candidates so the sample
/// is stable run over run.
fn sample_indices(n: usize, rate: f64) -> Vec<usize> {
    if n == 0 {
        return Vec::new();
    }
    let size = (n as f64 * rate).ceil() as usize;
    let size = size.clamp(n.min(5), n);
    (0..size).map(|i| i * n / size).collect()
}

fn run_review(ctx: &Ctx, results: &mut [RecordEval]) -> Result<ReviewReport, String> {
    let reviewer = ctx
        .config
        .reviewer
        .as_ref()
        .expect("check_pins ran")
        .clone();
    let mut candidates: Vec<usize> = results
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.review_candidate.then_some(i))
        .collect();
    candidates.sort_by(|&a, &b| {
        results[a]
            .outcome_record
            .record_id
            .cmp(&results[b].outcome_record.record_id)
    });
    let sampled: Vec<usize> = sample_indices(candidates.len(), ctx.config.sample_rate)
        .into_iter()
        .map(|i| candidates[i])
        .collect();

    let mut report = ReviewReport {
        sample_rate: ctx.config.sample_rate,
        sampled: sampled
            .iter()
            .map(|&i| results[i].outcome_record.record_id.clone())
            .collect(),
        results: Vec::new(),
    };

    // First pass: collect each sampled record's review verdict without
    // touching outcomes — the batch-level rule below decides together.
    let mut batch_passed = true;
    for &i in &sampled {
        let record = ctx
            .batch
            .records
            .iter()
            .find(|r| {
                r.record_id() == results[i].outcome_record.record_id && r.path == results[i].path
            })
            .expect("outcome records come from the batch");
        let push = |outcome: &str, detail: Option<String>| ReviewResult {
            record_id: record.record_id(),
            reviewer_run_id: reviewer.run_id.clone(),
            outcome: outcome.to_owned(),
            detail,
        };
        // The reviewer must not be the drafting run either.
        if reviewer
            .model_id
            .as_deref()
            .is_some_and(|m| same_model(ctx, record, m, None))
        {
            batch_passed = false;
            report.results.push(push(
                "held",
                Some("reviewer is the drafting model".to_owned()),
            ));
            continue;
        }
        let body = claims::review_body(&ctx.batch.batch_id, record);
        let outcome = match ctx.fulfillment.exchange(RequestKind::Review, &body) {
            Ok(Exchange::Response { value }) => value,
            Ok(Exchange::Unavailable { key }) => {
                batch_passed = false;
                report.results.push(push(
                    "unavailable",
                    Some(format!("review fulfillment unavailable ({key})")),
                ));
                continue;
            }
            Ok(Exchange::Malformed { key, detail }) => {
                batch_passed = false;
                report.results.push(push(
                    "malformed",
                    Some(format!(
                        "review response {key} is not usable JSON: {detail}"
                    )),
                ));
                continue;
            }
            Err(e) => return Err(format!("review fulfillment I/O: {e}")),
        };
        if let Some(verdict) = identity_ok(&outcome, &reviewer.run_id, None, "reviewer") {
            batch_passed = false;
            let reason = match verdict {
                Verdict::Hold(r) | Verdict::Reject(r) => r.join("; "),
                Verdict::Pass => String::new(),
            };
            report.results.push(push("identity-mismatch", Some(reason)));
            continue;
        }
        match outcome["outcome"].as_str() {
            Some("pass") => report
                .results
                .push(push("pass", outcome["detail"].as_str().map(str::to_owned))),
            Some(other) => {
                batch_passed = false;
                report
                    .results
                    .push(push(other, outcome["detail"].as_str().map(str::to_owned)));
            }
            None => {
                batch_passed = false;
                report.results.push(push(
                    "no-outcome",
                    Some("review response carries no `outcome`".to_owned()),
                ));
            }
        }
    }

    // The batch-level rule: a sample that does not pass holds the whole
    // batch — unaudited records are suspect too, so nothing is admitted.
    if batch_passed {
        for i in candidates {
            results[i].outcome_record.status = "admitted".to_owned();
        }
    } else {
        let reasons: HashMap<String, String> = report
            .results
            .iter()
            .map(|r| {
                (
                    r.record_id.clone(),
                    format!("{}: {}", r.outcome, r.detail.clone().unwrap_or_default()),
                )
            })
            .collect();
        for i in candidates {
            let id = &results[i].outcome_record.record_id;
            let reason = reasons
                .get(id)
                .cloned()
                .unwrap_or_else(|| "batch held: the sampled review did not pass".to_owned());
            hold(&mut results[i], &reason);
        }
    }
    Ok(report)
}

/// Mark a review-pool record held and move it out of the admitted set.
fn hold(eval: &mut RecordEval, reason: &str) {
    eval.outcome = Outcome::Held;
    eval.stage = Some(Stage::Review);
    eval.review_candidate = false;
    eval.outcome_record.outcome = Outcome::Held;
    eval.outcome_record.stage = Some(Stage::Review.into());
    eval.outcome_record.stage_name = Some(Stage::Review.name().to_owned());
    eval.outcome_record.reasons.push(reason.to_owned());
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
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl")
                || path.file_name().and_then(|n| n.to_str())
                    == Some(format!("{batch_id}.jsonl").as_str())
            {
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

fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
