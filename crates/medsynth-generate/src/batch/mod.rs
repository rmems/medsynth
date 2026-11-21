//! Batch orchestration: budget the cells, run both lanes, self-validate, and
//! write JSONL plus the manifest.
//!
//! The teacher lane is prepared once, before any teacher prompt is dispatched.
//! A pin or credential refusal does not stop the procedural lane: procedural
//! records are still written and the manifest records the teacher refusal.
//! The CLI turns a recorded refusal into a nonzero exit after writing.

mod anchors;
mod cells;
mod io;

pub(crate) use anchors::anchor_pack;
pub use io::check_output_dir;

use std::path::PathBuf;

use serde_json::Value;

use crate::config::{Lane, RunConfig, TargetCell, TeacherSelection};
use crate::error::GenerateError;
use crate::gold::GoldCorpus;
use crate::hash::{display_path, sha256_str, text_file_sha256};
use crate::manifest::{
    AnchorReceipt, BatchIdInputs, BatchManifest, CellReport, DocumentRef, DroppedDraft,
    MANIFEST_VERSION, RecordEntry, RightsEnvelope, TeacherLaneReport, TeacherPinRef, TeachersRef,
    Totals, anchor_receipts, batch_id, pin_summary,
};
use crate::matrix::{CoverageMatrix, domain_slug, tier_slug};
use crate::record::GeneratedRecord;
use crate::registry::SourceRegistry;
use crate::teacher::{PreparedTeacher, TeacherClient, TeacherError, TeacherPinRegistry, prepare};
use crate::validate::DraftValidator;

/// Paths a batch needs.
#[derive(Debug, Clone)]
pub struct PlanPaths {
    pub config: PathBuf,
    pub taxonomy: PathBuf,
    pub sources: PathBuf,
    pub teachers: PathBuf,
    pub schemas: PathBuf,
    pub gold: PathBuf,
}

/// Everything loaded and validated before generation starts.
pub struct BatchPlan {
    pub config: RunConfig,
    pub config_value: Value,
    pub config_sha256: String,
    pub matrix: CoverageMatrix,
    pub registry: SourceRegistry,
    pub pins: Option<TeacherPinRegistry>,
    pub pins_error: Option<String>,
    pub teachers_sha256: String,
    pub gold: GoldCorpus,
    pub validator: DraftValidator,
    pub paths: PlanPaths,
}

/// Which lanes a run executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneFilter {
    All,
    Procedural,
    Teacher,
}

/// Builds a client for a teacher that passed the pin and credential checks.
pub type TeacherClientFactory<'a> =
    dyn Fn(&PreparedTeacher) -> Result<Box<dyn TeacherClient>, TeacherError> + 'a;

/// Caller-supplied teacher environment and transport.
pub struct ExecuteOptions<'a> {
    pub lane_filter: LaneFilter,
    /// Credential lookup. Injected so tests never mutate process globals.
    pub env: &'a dyn Fn(&str) -> Option<String>,
    /// Construct a client for a teacher that passed the pin and credential
    /// checks. Production builds an HTTP client; tests supply fixtures.
    pub teacher_client_factory: &'a TeacherClientFactory<'a>,
}

/// One written draft, raw text included for byte-level checks.
pub struct OutputRecord {
    pub record: GeneratedRecord,
    pub raw: String,
    pub lane: Lane,
}

/// Result of a run, before writing.
pub struct BatchOutput {
    pub records: Vec<OutputRecord>,
    pub manifest: BatchManifest,
    /// Present when the teacher lane was requested but refused or failed.
    pub lane_error: Option<TeacherError>,
    /// Directories drafts must never write into (gold corpus and its parent
    /// `data/`). Checked again by `write_output`.
    pub(crate) protected_dirs: Vec<PathBuf>,
}

/// Directories drafts must never write into: the gold corpus and the `data/`
/// tree above it.
pub(crate) fn protected_dirs(paths: &PlanPaths) -> Vec<PathBuf> {
    let mut dirs = vec![paths.gold.clone()];
    if let Some(parent) = paths.gold.parent() {
        dirs.push(parent.to_path_buf());
    }
    dirs
}

/// Load and validate all batch inputs.
pub fn load_plan(
    paths: &PlanPaths,
    seed_override: Option<u64>,
) -> Result<BatchPlan, GenerateError> {
    let mut config = RunConfig::load(&paths.config)?;
    if let Some(seed) = seed_override {
        config.seed = seed;
    }
    let matrix = CoverageMatrix::load(&paths.taxonomy)?;
    if matrix.version != crate::TAXONOMY_VERSION {
        return Err(GenerateError::Config(format!(
            "generator supports taxonomy {} but {} is version {}",
            crate::TAXONOMY_VERSION,
            paths.taxonomy.display(),
            matrix.version
        )));
    }
    config
        .validate_against(&matrix)
        .map_err(|errors| GenerateError::Config(errors.join("; ")))?;
    let registry = SourceRegistry::load(&paths.sources)?;
    let gold = GoldCorpus::load(&paths.gold)?;
    let validator = DraftValidator::load(&paths.schemas)?;
    let (pins, pins_error) = match TeacherPinRegistry::load(&paths.teachers) {
        Ok(pins) => (Some(pins), None),
        Err(error) => (None, Some(error.to_string())),
    };
    let teachers_sha256 = match text_file_sha256(&paths.teachers) {
        Ok(hash) => hash,
        Err(_) => sha256_str(""),
    };
    let config_value = serde_json::to_value(&config)?;
    let config_sha256 = sha256_str(&serde_json::to_string(&config_value)?);
    Ok(BatchPlan {
        config,
        config_value,
        config_sha256,
        matrix,
        registry,
        pins,
        pins_error,
        teachers_sha256,
        gold,
        validator,
        paths: paths.clone(),
    })
}

/// Everything the cell loops accumulate.
#[derive(Default)]
pub(crate) struct BatchSinks {
    pub records: Vec<OutputRecord>,
    pub record_entries: Vec<RecordEntry>,
    pub receipts: Vec<AnchorReceipt>,
    pub envelopes: Vec<RightsEnvelope>,
    pub dropped: Vec<DroppedDraft>,
    pub dropped_self: usize,
    pub dropped_generation: usize,
}

/// One failed attempt, ready to record as a dropped draft.
pub(crate) struct FailedAttempt {
    pub id: Option<String>,
    pub stage: &'static str,
    /// True when the draft failed self-validation (vs failing to be built).
    pub self_validation: bool,
    pub reasons: Vec<String>,
}

impl BatchSinks {
    /// Record a written draft and its manifest entry + anchor receipts.
    fn push_written(
        &mut self,
        record: GeneratedRecord,
        raw: String,
        lane: Lane,
        registry: &SourceRegistry,
    ) -> Result<(), GenerateError> {
        let id = record.id().as_str().to_owned();
        let record_receipts = anchor_receipts(&id, &record.anchors(), registry)
            .map_err(|reason| GenerateError::Other(format!("record `{id}`: {reason}")))?;
        self.record_entries.push(RecordEntry {
            id,
            format: record.format().slug().to_owned(),
            domain: domain_slug(record.domain()).to_owned(),
            subdomain: crate::config::subdomain_slug(record.subdomain()).to_owned(),
            tier: tier_slug(record.tier()).to_owned(),
            lane: lane_label(lane).to_owned(),
            generator: record.provenance().generator.clone(),
            sha256: sha256_str(&raw),
            jsonl_file: format!("{}.jsonl", record.format().slug()),
        });
        self.receipts.extend(record_receipts);
        self.records.push(OutputRecord { record, raw, lane });
        Ok(())
    }

    /// Record a dropped draft and bump the matching counter.
    fn drop_draft(&mut self, cell: &TargetCell, lane: Lane, failure: FailedAttempt) {
        if failure.self_validation {
            self.dropped_self += 1;
        } else {
            self.dropped_generation += 1;
        }
        self.dropped.push(DroppedDraft {
            id: failure.id,
            format: cell.format.slug().to_owned(),
            domain: domain_slug(cell.domain).to_owned(),
            subdomain: cell.subdomain_slug().to_owned(),
            tier: tier_slug(cell.tier).to_owned(),
            lane: lane_label(lane).to_owned(),
            stage: failure.stage.to_owned(),
            reasons: failure.reasons,
        });
    }
}

/// A draft that survived its lane and self-validation.
pub(crate) struct ProducedDraft {
    pub record: GeneratedRecord,
    pub raw: String,
    pub envelope: Option<RightsEnvelope>,
}

/// The per-cell draft budget per taxonomy §5.2.
pub(super) struct Budget {
    pub(super) target: u32,
    pub(super) cap: u32,
    pub(super) dispatch: u32,
    /// First free attempt ordinal for this batch. Resolved + in-flight
    /// attempts already consumed ids 0..attempt_base, so replacement drafts
    /// always get fresh ids and fresh RNG streams.
    pub(super) attempt_base: u32,
    pub(super) budget_limited: bool,
    pub(super) escalated: bool,
}

impl Budget {
    pub(super) fn for_cell(plan: &BatchPlan, cell: &TargetCell) -> Self {
        let target = plan.matrix.target(cell.key()).unwrap_or(0);
        let cap = target.saturating_mul(3);
        let used = cell
            .resolved_attempts
            .saturating_add(cell.in_flight_attempts);
        let allowed = cap.saturating_sub(used);
        let dispatch = cell.count.min(allowed);
        // Taxonomy §5.2: escalation waits for in-flight drafts to resolve.
        // A cell whose last batch is still in verification is waiting, not
        // exhausted — escalate only once nothing is pending and the resolved
        // count alone has reached the cap.
        let escalated = dispatch == 0
            && cell.count > 0
            && cell.in_flight_attempts == 0
            && cell.resolved_attempts >= cap;
        Self {
            target,
            cap,
            dispatch,
            attempt_base: used,
            budget_limited: dispatch < cell.count,
            escalated,
        }
    }
}

/// State of the teacher lane after the startup gate.
struct TeacherLane {
    report: TeacherLaneReport,
    pin_ref: Option<TeacherPinRef>,
    prepared: Option<PreparedTeacher>,
    client: Option<Box<dyn TeacherClient>>,
    error: Option<TeacherError>,
}

/// Pin + credential + client construction, once per run.
fn prepare_teacher_lane(plan: &BatchPlan, options: &ExecuteOptions<'_>) -> TeacherLane {
    let mut lane = TeacherLane {
        report: TeacherLaneReport::not_requested(),
        pin_ref: None,
        prepared: None,
        client: None,
        error: None,
    };
    let needs_teacher = plan
        .config
        .cells
        .iter()
        .any(|cell| cell.lane == Lane::Teacher)
        && options.lane_filter != LaneFilter::Procedural;
    if !needs_teacher {
        return lane;
    }
    lane.report.pin_id = plan.config.teacher.as_ref().map(|t| t.pin_id.clone());
    let (Some(pins), Some(selection)) = (&plan.pins, &plan.config.teacher) else {
        lane.report.status = "refused".to_owned();
        lane.error = Some(TeacherError::PinParse(
            plan.pins_error
                .clone()
                .unwrap_or_else(|| "teacher pin registry is unavailable".to_owned()),
        ));
        return lane;
    };
    if let Some(pin) = pins.get(&selection.pin_id) {
        lane.pin_ref = Some(TeacherPinRef::from_pin(&pins.path, pin));
    }
    match prepare_teacher(pins, selection, options) {
        Ok((prepared, client)) => {
            lane.report.status = "ran".to_owned();
            lane.prepared = Some(prepared);
            lane.client = Some(client);
        }
        Err(error) => {
            lane.report.status = "refused".to_owned();
            lane.error = Some(error);
        }
    }
    lane
}

fn prepare_teacher(
    pins: &TeacherPinRegistry,
    selection: &TeacherSelection,
    options: &ExecuteOptions<'_>,
) -> Result<(PreparedTeacher, Box<dyn TeacherClient>), TeacherError> {
    let prepared = prepare(pins, &selection.pin_id, options.env)?;
    let client = (options.teacher_client_factory)(&prepared)?;
    Ok((prepared, client))
}

/// Finish the teacher lane report once all cells have run.
fn finish_teacher_report(lane: &mut TeacherLane) {
    if lane.report.status == "not_requested" {
        return;
    }
    if lane.error.is_some() {
        lane.report.status = if lane.prepared.is_some() {
            "error".to_owned()
        } else {
            "refused".to_owned()
        };
        lane.report.error = lane.error.as_ref().map(ToString::to_string);
    }
}

/// Run a batch in memory.
pub fn execute(
    plan: &BatchPlan,
    options: &ExecuteOptions<'_>,
) -> Result<BatchOutput, GenerateError> {
    let mut sinks = BatchSinks::default();
    let mut lane = prepare_teacher_lane(plan, options);
    let mut cells = Vec::new();
    let mut totals = Totals::default();

    for cell in &plan.config.cells {
        totals.requested += cell.count;
        let report = run_cell(plan, options, cell, &mut lane, &mut sinks)?;
        accumulate(&mut totals, &report);
        cells.push(report);
    }
    finish_teacher_report(&mut lane);

    let manifest = build_manifest(plan, &lane, cells, totals, &sinks);
    Ok(BatchOutput {
        records: sinks.records,
        manifest,
        lane_error: lane.error,
        protected_dirs: protected_dirs(&plan.paths),
    })
}

fn run_cell(
    plan: &BatchPlan,
    options: &ExecuteOptions<'_>,
    cell: &TargetCell,
    lane: &mut TeacherLane,
    sinks: &mut BatchSinks,
) -> Result<CellReport, GenerateError> {
    if !lane_included(cell.lane, options.lane_filter) {
        return Ok(filtered_report(plan, cell));
    }
    let budget = Budget::for_cell(plan, cell);
    let ctx = cells::CellCtx {
        plan,
        cell,
        budget: &budget,
    };
    match cell.lane {
        Lane::Procedural => cells::run_procedural_cell(&ctx, sinks),
        Lane::Teacher => match (lane.prepared.as_ref(), lane.client.as_deref()) {
            (Some(prepared), Some(client)) if lane.error.is_none() => {
                let teacher = cells::TeacherCtx { prepared, client };
                cells::run_teacher_cell(&ctx, &teacher, sinks, &mut lane.error)
            }
            _ => Ok(refused_report(cell, &budget, "lane_refused")),
        },
    }
}

fn build_manifest(
    plan: &BatchPlan,
    lane: &TeacherLane,
    cells: Vec<CellReport>,
    totals: Totals,
    sinks: &BatchSinks,
) -> BatchManifest {
    let id = batch_id(&BatchIdInputs {
        seed: plan.config.seed,
        config_sha256: &plan.config_sha256,
        taxonomy_sha256: &plan.matrix.sha256,
        sources_sha256: &plan.registry.file_sha256,
        gold_sha256: plan.gold.corpus_sha256(),
        pin_entry_sha256: lane
            .pin_ref
            .as_ref()
            .map(|pin| pin.pin_entry_sha256.as_str()),
    });
    BatchManifest {
        manifest_version: MANIFEST_VERSION,
        batch_id: id,
        seed: plan.config.seed,
        config_sha256: plan.config_sha256.clone(),
        config: plan.config_value.clone(),
        taxonomy: DocumentRef {
            path: display_path(&plan.paths.taxonomy),
            version: plan.matrix.version.clone(),
            sha256: plan.matrix.sha256.clone(),
        },
        sources: DocumentRef {
            path: display_path(&plan.paths.sources),
            version: plan.registry.registry_version.clone(),
            sha256: plan.registry.file_sha256.clone(),
        },
        teachers: TeachersRef {
            path: display_path(&plan.paths.teachers),
            sha256: plan.teachers_sha256.clone(),
            pins: plan
                .pins
                .as_ref()
                .map(|pins| pins.pins().map(pin_summary).collect())
                .unwrap_or_default(),
        },
        gold: DocumentRef {
            path: display_path(&plan.paths.gold),
            version: crate::CONTRACT_VERSION.to_owned(),
            sha256: plan.gold.corpus_sha256().to_owned(),
        },
        teacher_pin: lane.pin_ref.clone(),
        teacher_lane: lane.report.clone(),
        cells,
        totals,
        records: sinks.record_entries.clone(),
        dropped_by_self_validation: sinks.dropped_self,
        dropped_by_generation: sinks.dropped_generation,
        dropped: sinks.dropped.clone(),
        rights_envelopes: sinks.envelopes.clone(),
        anchor_receipts: sinks.receipts.clone(),
    }
}

fn lane_included(lane: Lane, filter: LaneFilter) -> bool {
    match filter {
        LaneFilter::All => true,
        LaneFilter::Procedural => lane == Lane::Procedural,
        LaneFilter::Teacher => lane == Lane::Teacher,
    }
}

pub(crate) fn cell_status(budget: &Budget, produced: u32, dropped: u32) -> String {
    if budget.escalated {
        "escalated"
    } else if budget.budget_limited {
        "budget_limited"
    } else if produced == 0 && dropped > 0 {
        "all_dropped"
    } else if produced == 0 {
        "no_drafts"
    } else {
        "produced"
    }
    .to_owned()
}

/// Per-lane outcome counts folded into the shared report shape.
pub(crate) struct CellOutcome<'a> {
    pub lane: &'a str,
    pub dispatched: u32,
    pub produced: u32,
    pub dropped: u32,
    pub status: String,
}

/// The report fields every cell shares; per-lane runs only vary
/// lane/counts/status.
pub(crate) fn cell_report(
    cell: &TargetCell,
    budget: &Budget,
    outcome: CellOutcome<'_>,
) -> CellReport {
    CellReport {
        domain: domain_slug(cell.domain).to_owned(),
        format: cell.format.slug().to_owned(),
        tier: tier_slug(cell.tier).to_owned(),
        subdomain: cell.subdomain_slug().to_owned(),
        lane: outcome.lane.to_owned(),
        matrix_target: budget.target,
        requested: cell.count,
        resolved_attempts: cell.resolved_attempts,
        in_flight_attempts: cell.in_flight_attempts,
        budget_cap: budget.cap,
        dispatched: outcome.dispatched,
        produced: outcome.produced,
        dropped: outcome.dropped,
        budget_limited: budget.budget_limited,
        escalated: budget.escalated,
        status: outcome.status,
    }
}

fn filtered_report(plan: &BatchPlan, cell: &TargetCell) -> CellReport {
    let budget = Budget::for_cell(plan, cell);
    cell_report(
        cell,
        &budget,
        CellOutcome {
            lane: lane_label(cell.lane),
            dispatched: 0,
            produced: 0,
            dropped: 0,
            status: "filtered_out".to_owned(),
        },
    )
}

pub(crate) fn refused_report(cell: &TargetCell, budget: &Budget, status: &str) -> CellReport {
    cell_report(
        cell,
        budget,
        CellOutcome {
            lane: lane_label(cell.lane),
            dispatched: 0,
            produced: 0,
            dropped: 0,
            status: status.to_owned(),
        },
    )
}

fn lane_label(lane: Lane) -> &'static str {
    match lane {
        Lane::Procedural => "procedural",
        Lane::Teacher => "teacher",
    }
}

fn accumulate(totals: &mut Totals, report: &CellReport) {
    totals.dispatched += report.dispatched;
    totals.produced += report.produced;
    totals.dropped += report.dropped;
}
