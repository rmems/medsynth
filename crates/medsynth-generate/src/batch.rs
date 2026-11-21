//! Batch orchestration: budget the cells, run both lanes, self-validate, and
//! write JSONL plus the manifest.
//!
//! The teacher lane is prepared once, before any teacher prompt is dispatched.
//! A pin or credential refusal does not stop the procedural lane: procedural
//! records are still written and the manifest records the teacher refusal.
//! The CLI turns a recorded refusal into a nonzero exit after writing.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::config::{Lane, RunConfig, TargetCell, TeacherSelection};
use crate::error::GenerateError;
use crate::format::Format;
use crate::gold::GoldCorpus;
use crate::hash::{display_path, file_sha256, sha256_str};
use crate::manifest::{
    AnchorReceipt, BatchManifest, CellReport, DocumentRef, DroppedDraft, MANIFEST_VERSION,
    RecordEntry, RightsEnvelope, TeacherLaneReport, TeacherPinRef, TeachersRef, Totals,
    anchor_receipts, batch_id, pin_summary,
};
use crate::matrix::{CoverageMatrix, domain_slug, tier_slug};
use crate::procedural;
use crate::record::{GeneratedRecord, record_id_for};
use crate::registry::SourceRegistry;
use crate::rng::SeededRng;
use crate::teacher::{
    AnchorPoint, PreparedTeacher, TeacherClient, TeacherError, TeacherPinRegistry, TeacherRequest,
    build_prompt, completion_tokens, parse_completion_json, prepare,
};
use crate::validate::{DraftValidator, ExpectedProvenance, ValidationContext};

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
    let teachers_sha256 = match file_sha256(&paths.teachers) {
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

/// Run a batch in memory.
pub fn execute(
    plan: &BatchPlan,
    options: &ExecuteOptions<'_>,
) -> Result<BatchOutput, GenerateError> {
    let mut records = Vec::new();
    let mut record_entries = Vec::new();
    let mut receipts = Vec::new();
    let mut envelopes = Vec::new();
    let mut dropped = Vec::new();
    let mut cells = Vec::new();
    let mut totals = Totals::default();
    let mut dropped_self = 0usize;
    let mut dropped_generation = 0usize;

    let needs_teacher = plan
        .config
        .cells
        .iter()
        .any(|cell| cell.lane == Lane::Teacher)
        && options.lane_filter != LaneFilter::Procedural;
    let mut teacher_lane = TeacherLaneReport::not_requested();
    let mut teacher_pin_ref = None;
    let mut teacher_error: Option<TeacherError> = None;
    let mut prepared: Option<PreparedTeacher> = None;
    let mut teacher_client: Option<Box<dyn TeacherClient>> = None;

    if needs_teacher {
        if let (Some(pins), Some(selection)) = (&plan.pins, &plan.config.teacher) {
            teacher_lane.pin_id = Some(selection.pin_id.clone());
            if let Some(pin) = pins.get(&selection.pin_id) {
                teacher_pin_ref = Some(TeacherPinRef::from_pin(&pins.path, pin));
            }
            match prepare_teacher(pins, selection, options) {
                Ok((prepared_teacher, client)) => {
                    prepared = Some(prepared_teacher);
                    teacher_client = Some(client);
                    teacher_lane.status = "ran".to_owned();
                }
                Err(error) => {
                    teacher_lane.status = "refused".to_owned();
                    teacher_error = Some(error);
                }
            }
        } else {
            teacher_lane.status = "refused".to_owned();
            teacher_lane.pin_id = plan.config.teacher.as_ref().map(|t| t.pin_id.clone());
            teacher_error = Some(TeacherError::PinParse(
                plan.pins_error
                    .clone()
                    .unwrap_or_else(|| "teacher pin registry is unavailable".to_owned()),
            ));
        }
    }

    for cell in &plan.config.cells {
        if !lane_included(cell.lane, options.lane_filter) {
            cells.push(filtered_report(plan, cell));
            totals.requested += cell.count;
            continue;
        }
        totals.requested += cell.count;
        let budget = Budget::for_cell(plan, cell);

        match cell.lane {
            Lane::Procedural => {
                let report = run_procedural_cell(
                    plan,
                    cell,
                    budget,
                    &mut records,
                    &mut record_entries,
                    &mut receipts,
                    &mut dropped,
                    &mut dropped_self,
                    &mut dropped_generation,
                )?;
                accumulate(&mut totals, &report);
                cells.push(report);
            }
            Lane::Teacher => {
                let report = if let (Some(prepared_teacher), Some(client)) =
                    (&prepared, teacher_client.as_deref())
                {
                    if teacher_error.is_some() {
                        refused_report(cell, budget, "lane_error")
                    } else {
                        run_teacher_cell(
                            plan,
                            cell,
                            budget,
                            prepared_teacher,
                            client,
                            &mut records,
                            &mut record_entries,
                            &mut receipts,
                            &mut envelopes,
                            &mut dropped,
                            &mut dropped_self,
                            &mut dropped_generation,
                            &mut teacher_error,
                        )?
                    }
                } else {
                    refused_report(cell, budget, "lane_refused")
                };
                accumulate(&mut totals, &report);
                cells.push(report);
            }
        }
    }

    if needs_teacher {
        if teacher_error.is_some() {
            teacher_lane.status = if prepared.is_some() {
                "error".to_owned()
            } else {
                "refused".to_owned()
            };
            teacher_lane.error = teacher_error.as_ref().map(ToString::to_string);
        } else if prepared.is_some() {
            teacher_lane.status = "ran".to_owned();
        }
    }

    let manifest = BatchManifest {
        manifest_version: MANIFEST_VERSION,
        batch_id: batch_id(
            plan.config.seed,
            &plan.config_sha256,
            &plan.matrix.sha256,
            &plan.registry.file_sha256,
            teacher_pin_ref
                .as_ref()
                .map(|pin| pin.pin_entry_sha256.as_str()),
        ),
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
        teacher_pin: teacher_pin_ref,
        teacher_lane,
        cells,
        totals,
        records: record_entries,
        dropped_by_self_validation: dropped_self,
        dropped_by_generation: dropped_generation,
        dropped,
        rights_envelopes: envelopes,
        anchor_receipts: receipts,
    };
    Ok(BatchOutput {
        records,
        manifest,
        lane_error: teacher_error,
    })
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

fn lane_included(lane: Lane, filter: LaneFilter) -> bool {
    match filter {
        LaneFilter::All => true,
        LaneFilter::Procedural => lane == Lane::Procedural,
        LaneFilter::Teacher => lane == Lane::Teacher,
    }
}

struct Budget {
    target: u32,
    cap: u32,
    dispatch: u32,
    budget_limited: bool,
    escalated: bool,
}

impl Budget {
    fn for_cell(plan: &BatchPlan, cell: &TargetCell) -> Self {
        let target = plan.matrix.target(cell.key()).unwrap_or(0);
        let cap = target.saturating_mul(3);
        let used = cell
            .resolved_attempts
            .saturating_add(cell.in_flight_attempts);
        let allowed = cap.saturating_sub(used);
        let dispatch = cell.count.min(allowed);
        Self {
            target,
            cap,
            dispatch,
            budget_limited: dispatch < cell.count,
            escalated: dispatch == 0 && cell.count > 0 && used >= cap,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_procedural_cell(
    plan: &BatchPlan,
    cell: &TargetCell,
    budget: Budget,
    records: &mut Vec<OutputRecord>,
    record_entries: &mut Vec<RecordEntry>,
    receipts: &mut Vec<AnchorReceipt>,
    dropped: &mut Vec<DroppedDraft>,
    dropped_self: &mut usize,
    dropped_generation: &mut usize,
) -> Result<CellReport, GenerateError> {
    let template = cell.template.as_deref().unwrap_or_default();
    if let Err(reason) = procedural::check_template_for_cell(template, cell) {
        return Err(GenerateError::Config(format!(
            "cell `{}`: {reason}",
            cell.label()
        )));
    }
    let mut produced = 0u32;
    let mut cell_dropped = 0u32;
    let mut dispatched = 0u32;
    for index in 0..budget.dispatch {
        dispatched += 1;
        let id = match record_id_for(plan.config.seed, cell, index) {
            Ok(id) => id,
            Err(error) => {
                drop_draft(
                    dropped,
                    cell,
                    Lane::Procedural,
                    None,
                    "generation",
                    vec![format!("record id: {error}")],
                );
                cell_dropped += 1;
                *dropped_generation += 1;
                continue;
            }
        };
        let id_text = id.as_str().to_owned();
        let mut rng = SeededRng::new(
            plan.config.seed,
            &[cell.label().as_str(), template, &index.to_string()],
        );
        let record = match procedural::build(template, cell, id, &mut rng) {
            Ok(record) => record,
            Err(error) => {
                drop_draft(
                    dropped,
                    cell,
                    Lane::Procedural,
                    Some(id_text),
                    "generation",
                    vec![error],
                );
                cell_dropped += 1;
                *dropped_generation += 1;
                continue;
            }
        };
        let raw = match record.to_json_string() {
            Ok(raw) => raw,
            Err(error) => {
                drop_draft(
                    dropped,
                    cell,
                    Lane::Procedural,
                    Some(record.id().as_str().to_owned()),
                    "generation",
                    vec![format!("serialize: {error}")],
                );
                cell_dropped += 1;
                *dropped_generation += 1;
                continue;
            }
        };
        let context = ValidationContext {
            expected: ExpectedProvenance::Procedural,
            registry: &plan.registry,
            anchor_allowlist: None,
        };
        match plan.validator.validate_raw(record.format(), &raw, &context) {
            Ok(()) => {
                push_written(
                    records,
                    record_entries,
                    receipts,
                    record,
                    raw,
                    Lane::Procedural,
                    &plan.registry,
                )?;
                produced += 1;
            }
            Err(errors) => {
                drop_draft(
                    dropped,
                    cell,
                    Lane::Procedural,
                    Some(record.id().as_str().to_owned()),
                    "self_validation",
                    errors,
                );
                cell_dropped += 1;
                *dropped_self += 1;
            }
        }
    }
    Ok(CellReport {
        domain: domain_slug(cell.domain).to_owned(),
        format: cell.format.slug().to_owned(),
        tier: tier_slug(cell.tier).to_owned(),
        subdomain: cell.subdomain_slug().to_owned(),
        lane: "procedural".to_owned(),
        matrix_target: budget.target,
        requested: cell.count,
        resolved_attempts: cell.resolved_attempts,
        in_flight_attempts: cell.in_flight_attempts,
        budget_cap: budget.cap,
        dispatched,
        produced,
        dropped: cell_dropped,
        budget_limited: budget.budget_limited,
        escalated: budget.escalated,
        status: cell_status(&budget, produced, cell_dropped),
    })
}

#[allow(clippy::too_many_arguments)]
fn run_teacher_cell(
    plan: &BatchPlan,
    cell: &TargetCell,
    budget: Budget,
    prepared: &PreparedTeacher,
    client: &dyn TeacherClient,
    records: &mut Vec<OutputRecord>,
    record_entries: &mut Vec<RecordEntry>,
    receipts: &mut Vec<AnchorReceipt>,
    envelopes: &mut Vec<RightsEnvelope>,
    dropped: &mut Vec<DroppedDraft>,
    dropped_self: &mut usize,
    dropped_generation: &mut usize,
    teacher_error: &mut Option<TeacherError>,
) -> Result<CellReport, GenerateError> {
    let selection = plan
        .config
        .teacher
        .as_ref()
        .ok_or_else(|| GenerateError::Config("teacher selection missing".to_owned()))?;
    let anchors = anchor_pack(plan, cell);
    let allowlist: BTreeSet<(String, String)> = anchors
        .iter()
        .map(|anchor| (anchor.source_id.clone(), anchor.locator.clone()))
        .collect();
    let exemplars = plan
        .gold
        .exemplars(cell.format, cell.domain, selection.exemplars_per_prompt);
    let prompt = match build_prompt(cell, &anchors, &exemplars, selection) {
        Ok(prompt) => prompt,
        Err(error) => {
            *teacher_error = Some(error);
            return Ok(refused_report(cell, budget, "lane_error"));
        }
    };

    let mut produced = 0u32;
    let mut cell_dropped = 0u32;
    let mut dispatched = 0u32;
    for index in 0..budget.dispatch {
        dispatched += 1;
        let id = match record_id_for(plan.config.seed, cell, index) {
            Ok(id) => id,
            Err(error) => {
                drop_draft(
                    dropped,
                    cell,
                    Lane::Teacher,
                    None,
                    "generation",
                    vec![format!("record id: {error}")],
                );
                cell_dropped += 1;
                *dropped_generation += 1;
                continue;
            }
        };
        let id_text = id.as_str().to_owned();
        let request = TeacherRequest {
            model: prepared.pin().model_id.clone(),
            system: prompt.system.clone(),
            user: prompt.user.clone(),
            max_tokens: selection.max_output_tokens,
            temperature: selection.temperature,
        };
        let completion = match client.complete(&request) {
            Ok(completion) => completion,
            Err(error) => {
                drop_draft(
                    dropped,
                    cell,
                    Lane::Teacher,
                    Some(id_text),
                    "teacher_transport",
                    vec![error.to_string()],
                );
                cell_dropped += 1;
                *dropped_generation += 1;
                continue;
            }
        };
        let actual_tokens = completion_tokens(&completion.content);
        if actual_tokens > selection.max_output_tokens {
            drop_draft(
                dropped,
                cell,
                Lane::Teacher,
                Some(id_text),
                "teacher_budget",
                vec![format!(
                    "completion used {actual_tokens} tokens; limit {}",
                    selection.max_output_tokens
                )],
            );
            cell_dropped += 1;
            *dropped_generation += 1;
            continue;
        }
        let content = match parse_completion_json(cell.format, &completion.content) {
            Ok(content) => content,
            Err(error) => {
                drop_draft(
                    dropped,
                    cell,
                    Lane::Teacher,
                    Some(id_text),
                    "teacher_parse",
                    vec![error.to_string()],
                );
                cell_dropped += 1;
                *dropped_generation += 1;
                continue;
            }
        };
        let record = match crate::teacher::build_record(
            cell.format,
            content,
            cell,
            id,
            prepared.pin(),
            &prompt.prompt_version,
        ) {
            Ok(record) => record,
            Err(error) => {
                drop_draft(
                    dropped,
                    cell,
                    Lane::Teacher,
                    Some(id_text),
                    "teacher_parse",
                    vec![error.to_string()],
                );
                cell_dropped += 1;
                *dropped_generation += 1;
                continue;
            }
        };
        let raw = match record.to_json_string() {
            Ok(raw) => raw,
            Err(error) => {
                drop_draft(
                    dropped,
                    cell,
                    Lane::Teacher,
                    Some(record.id().as_str().to_owned()),
                    "generation",
                    vec![format!("serialize: {error}")],
                );
                cell_dropped += 1;
                *dropped_generation += 1;
                continue;
            }
        };
        let record_id = record.id().as_str().to_owned();
        let envelope = match RightsEnvelope::build(&record_id, &raw, prepared.pin()) {
            Ok(envelope) => envelope,
            Err(reason) => {
                drop_draft(
                    dropped,
                    cell,
                    Lane::Teacher,
                    Some(record_id),
                    "rights_envelope",
                    vec![reason],
                );
                cell_dropped += 1;
                *dropped_self += 1;
                continue;
            }
        };
        let context = ValidationContext {
            expected: ExpectedProvenance::Teacher {
                pin: prepared.pin(),
                prompt_version: &prompt.prompt_version,
            },
            registry: &plan.registry,
            anchor_allowlist: Some(&allowlist),
        };
        match plan.validator.validate_raw(cell.format, &raw, &context) {
            Ok(()) => {
                push_written(
                    records,
                    record_entries,
                    receipts,
                    record,
                    raw,
                    Lane::Teacher,
                    &plan.registry,
                )?;
                envelopes.push(envelope);
                produced += 1;
            }
            Err(errors) => {
                drop_draft(
                    dropped,
                    cell,
                    Lane::Teacher,
                    Some(record_id),
                    "self_validation",
                    errors,
                );
                cell_dropped += 1;
                *dropped_self += 1;
            }
        }
    }
    Ok(CellReport {
        domain: domain_slug(cell.domain).to_owned(),
        format: cell.format.slug().to_owned(),
        tier: tier_slug(cell.tier).to_owned(),
        subdomain: cell.subdomain_slug().to_owned(),
        lane: "teacher".to_owned(),
        matrix_target: budget.target,
        requested: cell.count,
        resolved_attempts: cell.resolved_attempts,
        in_flight_attempts: cell.in_flight_attempts,
        budget_cap: budget.cap,
        dispatched,
        produced,
        dropped: cell_dropped,
        budget_limited: budget.budget_limited,
        escalated: budget.escalated,
        status: cell_status(&budget, produced, cell_dropped),
    })
}

fn cell_status(budget: &Budget, produced: u32, dropped: u32) -> String {
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

fn filtered_report(plan: &BatchPlan, cell: &TargetCell) -> CellReport {
    let budget = Budget::for_cell(plan, cell);
    CellReport {
        domain: domain_slug(cell.domain).to_owned(),
        format: cell.format.slug().to_owned(),
        tier: tier_slug(cell.tier).to_owned(),
        subdomain: cell.subdomain_slug().to_owned(),
        lane: lane_label(cell.lane).to_owned(),
        matrix_target: budget.target,
        requested: cell.count,
        resolved_attempts: cell.resolved_attempts,
        in_flight_attempts: cell.in_flight_attempts,
        budget_cap: budget.cap,
        dispatched: 0,
        produced: 0,
        dropped: 0,
        budget_limited: false,
        escalated: false,
        status: "filtered_out".to_owned(),
    }
}

fn refused_report(cell: &TargetCell, budget: Budget, status: &str) -> CellReport {
    CellReport {
        domain: domain_slug(cell.domain).to_owned(),
        format: cell.format.slug().to_owned(),
        tier: tier_slug(cell.tier).to_owned(),
        subdomain: cell.subdomain_slug().to_owned(),
        lane: lane_label(cell.lane).to_owned(),
        matrix_target: budget.target,
        requested: cell.count,
        resolved_attempts: cell.resolved_attempts,
        in_flight_attempts: cell.in_flight_attempts,
        budget_cap: budget.cap,
        dispatched: 0,
        produced: 0,
        dropped: 0,
        budget_limited: false,
        escalated: budget.escalated,
        status: status.to_owned(),
    }
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

fn drop_draft(
    dropped: &mut Vec<DroppedDraft>,
    cell: &TargetCell,
    lane: Lane,
    id: Option<String>,
    stage: &str,
    reasons: Vec<String>,
) {
    dropped.push(DroppedDraft {
        id,
        format: cell.format.slug().to_owned(),
        domain: domain_slug(cell.domain).to_owned(),
        subdomain: cell.subdomain_slug().to_owned(),
        tier: tier_slug(cell.tier).to_owned(),
        lane: lane_label(lane).to_owned(),
        stage: stage.to_owned(),
        reasons,
    });
}

fn push_written(
    records: &mut Vec<OutputRecord>,
    record_entries: &mut Vec<RecordEntry>,
    receipts: &mut Vec<AnchorReceipt>,
    record: GeneratedRecord,
    raw: String,
    lane: Lane,
    registry: &SourceRegistry,
) -> Result<(), GenerateError> {
    let id = record.id().as_str().to_owned();
    let record_receipts = anchor_receipts(&id, &record.anchors(), registry)
        .map_err(|reason| GenerateError::Other(format!("record `{id}`: {reason}")))?;
    record_entries.push(RecordEntry {
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
    receipts.extend(record_receipts);
    records.push(OutputRecord { record, raw, lane });
    Ok(())
}

/// Build the approved anchor list for a teacher cell.
///
/// Order is deterministic: fact-catalog entries for the subdomain, then gold
/// anchors for the same format and domain, then one registry locator example
/// per current source covering the subdomain.
fn anchor_pack(plan: &BatchPlan, cell: &TargetCell) -> Vec<AnchorPoint> {
    let mut points = Vec::new();
    for fact in procedural::FACTS
        .iter()
        .filter(|fact| fact.domain == cell.domain && fact.subdomain == cell.subdomain)
    {
        let text = format!("{} Answer: {}", fact.stem, fact.correct);
        for (source_id, locator) in fact.anchors {
            push_point(&mut points, source_id, locator, &text);
        }
    }
    let slug = domain_slug(cell.domain);
    for gold in plan.gold.records(cell.format) {
        if gold.get("domain").and_then(Value::as_str) != Some(slug) {
            continue;
        }
        let fact = gold_text(gold);
        let mut anchors = Vec::new();
        collect_gold_anchors(gold, &mut anchors);
        for (source_id, locator) in anchors {
            push_point(&mut points, &source_id, &locator, &fact);
        }
    }
    for source in plan.registry.current_covering(cell.subdomain_slug()) {
        if let Some(example) = source.locator_convention.examples.first() {
            push_point(&mut points, &source.source_id, example, "");
        }
    }
    points.truncate(16);
    points
}

fn push_point(points: &mut Vec<AnchorPoint>, source_id: &str, locator: &str, fact: &str) {
    if points
        .iter()
        .any(|point| point.source_id == source_id && point.locator == locator)
    {
        return;
    }
    points.push(AnchorPoint {
        source_id: source_id.to_owned(),
        locator: locator.to_owned(),
        fact: fact.to_owned(),
    });
}

fn collect_gold_anchors(value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(object) => {
            if let (Some(source_id), Some(locator)) = (
                object.get("source_id").and_then(Value::as_str),
                object.get("locator").and_then(Value::as_str),
            ) {
                out.push((source_id.to_owned(), locator.to_owned()));
            }
            for child in object.values() {
                collect_gold_anchors(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_gold_anchors(item, out);
            }
        }
        _ => {}
    }
}

fn gold_text(value: &Value) -> String {
    for key in ["stem", "learner_prompt"] {
        if let Some(text) = value
            .get(key)
            .and_then(|block| block.get("text"))
            .and_then(Value::as_str)
        {
            return truncate(text, 240);
        }
    }
    if let Some(text) = value.get("presentation").and_then(Value::as_str) {
        return truncate(text, 240);
    }
    if let Some(text) = value
        .get("tutor_response")
        .and_then(|block| block.get("text"))
        .and_then(Value::as_str)
    {
        return truncate(text, 240);
    }
    String::new()
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

impl BatchOutput {
    /// Write one JSONL shard per produced format plus `manifest.json`.
    ///
    /// The output directory may not be under `data/`: admitted records belong
    /// in `data/<format>/` only after #9 admission, and drafts must never be
    /// written into `data/gold/`.
    pub fn write_output(&self, out_dir: &Path) -> Result<Vec<PathBuf>, GenerateError> {
        assert_safe_output_dir(out_dir)?;
        std::fs::create_dir_all(out_dir)
            .map_err(|e| GenerateError::io(out_dir.to_path_buf(), e))?;
        let mut grouped: BTreeMap<Format, Vec<&OutputRecord>> = BTreeMap::new();
        for record in &self.records {
            grouped
                .entry(record.record.format())
                .or_default()
                .push(record);
        }
        let mut written = Vec::new();
        for (format, records) in grouped {
            let path = out_dir.join(format!("{}.jsonl", format.slug()));
            let mut text = String::new();
            for record in records {
                text.push_str(&record.raw);
                text.push('\n');
            }
            std::fs::write(&path, text).map_err(|e| GenerateError::io(path.clone(), e))?;
            written.push(path);
        }
        let manifest_path = out_dir.join("manifest.json");
        std::fs::write(&manifest_path, self.manifest.to_json_string()?)
            .map_err(|e| GenerateError::io(manifest_path.clone(), e))?;
        written.push(manifest_path);
        Ok(written)
    }
}

fn assert_safe_output_dir(out_dir: &Path) -> Result<(), GenerateError> {
    let normalized = display_path(out_dir);
    if normalized.split('/').any(|component| component == "data") {
        return Err(GenerateError::Config(format!(
            "refusing to write drafts under `data/` (got `{normalized}`); admitted records belong in data/<format>/ only after #9"
        )));
    }
    Ok(())
}
