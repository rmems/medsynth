//! Per-cell lane runners. Each attempt either produces a draft or records a
//! dropped draft with the stage that failed — a single attempt failure never
//! aborts the batch.

use std::collections::BTreeSet;

use crate::batch::{
    BatchPlan, BatchSinks, Budget, CellOutcome, FailedAttempt, OutputRecord, ProducedDraft,
    TeacherStats, anchor_pack, cell_report, cell_status,
};
use crate::config::{Lane, TargetCell, TeacherSelection};
use crate::error::GenerateError;
use crate::manifest::{CellReport, RightsEnvelope};
use crate::procedural;
use crate::record::record_id_for;
use crate::rng::SeededRng;
use crate::teacher::{
    BuiltPrompt, PreparedTeacher, RecordStamp, TeacherClient, TeacherCompletion, TeacherError,
    TeacherRequest, build_prompt, completion_tokens, parse_completion_json,
};
use crate::validate::{ExpectedProvenance, ValidationContext};

/// Everything a cell runner needs: the plan, the cell, and its budget.
pub(crate) struct CellCtx<'a> {
    pub plan: &'a BatchPlan,
    pub cell: &'a TargetCell,
    pub budget: &'a Budget,
}

/// The teacher that passed the startup gate, plus its transport.
pub(crate) struct TeacherCtx<'a> {
    pub prepared: &'a PreparedTeacher,
    pub client: &'a dyn TeacherClient,
}

/// Run the procedural lane for one cell.
pub(super) fn run_procedural_cell(
    ctx: &CellCtx<'_>,
    sinks: &mut BatchSinks,
) -> Result<CellReport, GenerateError> {
    let cell = ctx.cell;
    let template = cell.template.as_deref().unwrap_or_default();
    if let Err(reason) = procedural::check_template_for_cell(template, cell) {
        return Err(GenerateError::Config(format!(
            "cell `{}`: {reason}",
            cell.label()
        )));
    }
    let mut dispatched = 0u32;
    let mut produced = 0u32;
    let mut cell_dropped = 0u32;
    for index in 0..ctx.budget.dispatch {
        dispatched += 1;
        match procedural_attempt(ctx, template, ctx.budget.attempt_base + index) {
            Ok(draft) => {
                sinks.push_written(
                    OutputRecord {
                        record: draft.record,
                        raw: draft.raw,
                        lane: Lane::Procedural,
                    },
                    &ctx.plan.registry,
                    ctx.plan.config.seed,
                )?;
                produced += 1;
            }
            Err(failure) => {
                sinks.drop_draft(cell, Lane::Procedural, failure);
                cell_dropped += 1;
            }
        }
    }
    Ok(cell_report(
        cell,
        ctx.budget,
        CellOutcome {
            lane: "procedural",
            dispatched,
            produced,
            dropped: cell_dropped,
            status: cell_status(ctx.budget, produced, cell_dropped),
        },
    ))
}

fn procedural_attempt(
    ctx: &CellCtx<'_>,
    template: &str,
    attempt: u32,
) -> Result<ProducedDraft, FailedAttempt> {
    let fail = |id, stage, reasons: Vec<String>| FailedAttempt {
        id,
        stage,
        self_validation: stage == "self_validation",
        reasons,
    };
    let id = record_id_for(ctx.plan.config.seed, ctx.cell, attempt)
        .map_err(|e| fail(None, "generation", vec![format!("record id: {e}")]))?;
    let id_text = id.as_str().to_owned();
    let mut rng = SeededRng::new(
        ctx.plan.config.seed,
        &[ctx.cell.label().as_str(), template, &attempt.to_string()],
    );
    let record = procedural::build(template, ctx.cell, id, &mut rng)
        .map_err(|e| fail(Some(id_text.clone()), "generation", vec![e]))?;
    let raw = record.to_json_string().map_err(|e| {
        fail(
            Some(record.id().as_str().to_owned()),
            "generation",
            vec![format!("serialize: {e}")],
        )
    })?;
    let context = ValidationContext {
        expected: ExpectedProvenance::Procedural,
        registry: &ctx.plan.registry,
        anchor_allowlist: None,
    };
    ctx.plan
        .validator
        .validate_raw(record.format(), &raw, &context)
        .map_err(|errors| {
            fail(
                Some(record.id().as_str().to_owned()),
                "self_validation",
                errors,
            )
        })?;
    Ok(ProducedDraft {
        record,
        raw,
        envelope: None,
    })
}

/// Run the teacher lane for one cell.
///
/// Transport counters accumulate lane-wide: a cell whose attempts all failed
/// at transport does not stop the next cell — the lane fails only when every
/// request across the run fails at transport (see `finish_teacher_report`).
pub(super) fn run_teacher_cell(
    ctx: &CellCtx<'_>,
    teacher: &TeacherCtx<'_>,
    sinks: &mut BatchSinks,
    stats: &mut TeacherStats,
) -> Result<CellReport, GenerateError> {
    let cell = ctx.cell;
    let setup = match teacher_setup(ctx) {
        Ok(setup) => setup,
        Err(error) => {
            stats.error = Some(error);
            return Ok(crate::batch::refused_report(cell, ctx.budget, "lane_error"));
        }
    };
    let run = TeacherRun {
        ctx,
        teacher,
        setup: &setup,
    };
    let (produced, cell_dropped) = dispatch_teacher_attempts(&run, sinks, stats)?;
    Ok(cell_report(
        cell,
        ctx.budget,
        CellOutcome {
            lane: "teacher",
            dispatched: ctx.budget.dispatch,
            produced,
            dropped: cell_dropped,
            status: cell_status(ctx.budget, produced, cell_dropped),
        },
    ))
}

/// Everything a teacher cell needs to dispatch requests.
struct TeacherSetup<'a> {
    selection: &'a TeacherSelection,
    allowlist: BTreeSet<(String, String)>,
    prompt: BuiltPrompt,
}

/// Build the anchor pack, allowlist, and prompt for the cell. A prompt-build
/// failure is reported as a lane error by the caller.
fn teacher_setup<'a>(ctx: &'a CellCtx<'a>) -> Result<TeacherSetup<'a>, TeacherError> {
    let cell = ctx.cell;
    let selection = ctx
        .plan
        .config
        .teacher
        .as_ref()
        .ok_or_else(|| TeacherError::Parse("teacher selection missing".to_owned()))?;
    let anchors = anchor_pack(ctx.plan, cell);
    let allowlist: BTreeSet<(String, String)> = anchors
        .iter()
        .map(|anchor| (anchor.source_id.clone(), anchor.locator.clone()))
        .collect();
    let exemplars =
        ctx.plan
            .gold
            .exemplars(cell.format, cell.domain, selection.exemplars_per_prompt);
    let prompt = build_prompt(cell, &anchors, &exemplars, selection)?;
    Ok(TeacherSetup {
        selection,
        allowlist,
        prompt,
    })
}

/// The fixed inputs every attempt in a teacher cell shares.
struct TeacherRun<'a> {
    ctx: &'a CellCtx<'a>,
    teacher: &'a TeacherCtx<'a>,
    setup: &'a TeacherSetup<'a>,
}

/// Dispatch every attempt the budget allows, folding outcomes into the sinks
/// and the lane-wide transport counters. Returns (produced, dropped).
fn dispatch_teacher_attempts(
    run: &TeacherRun<'_>,
    sinks: &mut BatchSinks,
    stats: &mut TeacherStats,
) -> Result<(u32, u32), GenerateError> {
    let ctx = run.ctx;
    let mut produced = 0u32;
    let mut cell_dropped = 0u32;
    for index in 0..ctx.budget.dispatch {
        stats.dispatched += 1;
        let attempt = AttemptCtx {
            cell_ctx: ctx,
            attempt: ctx.budget.attempt_base + index,
            teacher: run.teacher,
            selection: run.setup.selection,
            prompt: &run.setup.prompt,
            allowlist: &run.setup.allowlist,
        };
        match teacher_attempt(&attempt) {
            Ok(draft) => {
                sinks.push_written(
                    OutputRecord {
                        record: draft.record,
                        raw: draft.raw,
                        lane: Lane::Teacher,
                    },
                    &ctx.plan.registry,
                    ctx.plan.config.seed,
                )?;
                if let Some(envelope) = draft.envelope {
                    sinks.envelopes.push(envelope);
                }
                produced += 1;
            }
            Err(failure) => {
                if failure.stage == "teacher_transport" {
                    stats.transport_failures += 1;
                }
                sinks.drop_draft(ctx.cell, Lane::Teacher, failure);
                cell_dropped += 1;
            }
        }
    }
    Ok((produced, cell_dropped))
}

/// Inputs one teacher attempt needs, bundled to keep the signature small.
struct AttemptCtx<'a> {
    cell_ctx: &'a CellCtx<'a>,
    attempt: u32,
    teacher: &'a TeacherCtx<'a>,
    selection: &'a TeacherSelection,
    prompt: &'a BuiltPrompt,
    allowlist: &'a BTreeSet<(String, String)>,
}

impl AttemptCtx<'_> {
    fn fail(&self, id: Option<String>, stage: &'static str, reasons: Vec<String>) -> FailedAttempt {
        FailedAttempt {
            id,
            stage,
            self_validation: matches!(stage, "self_validation" | "rights_envelope"),
            reasons,
        }
    }
}

fn teacher_attempt(ctx: &AttemptCtx<'_>) -> Result<ProducedDraft, FailedAttempt> {
    let cell = ctx.cell_ctx.cell;
    let id = record_id_for(ctx.cell_ctx.plan.config.seed, cell, ctx.attempt)
        .map_err(|e| ctx.fail(None, "generation", vec![format!("record id: {e}")]))?;
    let completion = request_completion(ctx, id.as_str())?;
    let content = parse_completion_json(cell.format, &completion.content).map_err(|e| {
        ctx.fail(
            Some(id.as_str().to_owned()),
            "teacher_parse",
            vec![e.to_string()],
        )
    })?;
    finalize_draft(
        ctx,
        id,
        content,
        completion.served_model.as_deref(),
        completion.served_vendor.as_deref(),
    )
}

/// Dispatch the request. Transport failures (network/HTTP) stay distinct from
/// response-shape failures like `MissingContent` so only real transport
/// outages count toward the lane failure check.
fn request_completion(
    ctx: &AttemptCtx<'_>,
    id_text: &str,
) -> Result<TeacherCompletion, FailedAttempt> {
    let pin = ctx.teacher.prepared.pin();
    let request = TeacherRequest {
        model: pin.model_id.clone(),
        system: ctx.prompt.system.clone(),
        user: ctx.prompt.user.clone(),
        max_tokens: ctx.selection.max_output_tokens,
        temperature: ctx.selection.temperature,
        // Fold every batch-seed bit into the API seed so a new --seed also
        // asks the teacher for a fresh sample, not a replayed one. The fold
        // needn't be injective — the fulfillment key is unique via
        // metadata.batch_seed; only the sampling varies here.
        seed: ((ctx.cell_ctx.plan.config.seed ^ (ctx.cell_ctx.plan.config.seed >> 32)) as u32)
            .wrapping_add(ctx.attempt),
        // Carried in `metadata` so a shared fulfillment directory serves
        // fresh samples per batch, not cross-batch dupes.
        batch_seed: ctx.cell_ctx.plan.config.seed,
        pin_id: pin.pin_id.clone(),
        pin_variant: pin.variant.clone(),
        include_routing_metadata: pin.endpoint.contains("api-gateway.merge.dev"),
    };
    let completion = ctx.teacher.client.complete(&request).map_err(|e| {
        let stage = if matches!(e, TeacherError::Transport(_)) {
            "teacher_transport"
        } else {
            "teacher_response"
        };
        ctx.fail(Some(id_text.to_owned()), stage, vec![e.to_string()])
    })?;
    let actual_tokens = completion_tokens(&completion.content);
    if actual_tokens > ctx.selection.max_output_tokens {
        return Err(ctx.fail(
            Some(id_text.to_owned()),
            "teacher_budget",
            vec![format!(
                "completion used {actual_tokens} tokens; limit {}",
                ctx.selection.max_output_tokens
            )],
        ));
    }
    Ok(completion)
}

/// Build the record from parsed content, wrap it in a rights envelope, and
/// self-validate before it may be written.
fn finalize_draft(
    ctx: &AttemptCtx<'_>,
    id: medsynth_schema::RecordId,
    content: crate::teacher::TeacherContent,
    served_model: Option<&str>,
    served_vendor: Option<&str>,
) -> Result<ProducedDraft, FailedAttempt> {
    let cell = ctx.cell_ctx.cell;
    let plan = ctx.cell_ctx.plan;
    let stamp = RecordStamp {
        id,
        pin: ctx.teacher.prepared.pin(),
        prompt_version: &ctx.prompt.prompt_version,
    };
    let record = crate::teacher::build_record(cell.format, content, cell, &stamp).map_err(|e| {
        ctx.fail(
            Some(stamp.id.as_str().to_owned()),
            "teacher_parse",
            vec![e.to_string()],
        )
    })?;
    let raw = record.to_json_string().map_err(|e| {
        ctx.fail(
            Some(record.id().as_str().to_owned()),
            "generation",
            vec![format!("serialize: {e}")],
        )
    })?;
    let record_id = record.id().as_str().to_owned();
    let envelope = RightsEnvelope::build(
        &record_id,
        &raw,
        ctx.teacher.prepared.pin(),
        served_model,
        served_vendor,
    )
    .map_err(|reason| ctx.fail(Some(record_id.clone()), "rights_envelope", vec![reason]))?;
    let context = ValidationContext {
        expected: ExpectedProvenance::Teacher {
            pin: ctx.teacher.prepared.pin(),
            prompt_version: &ctx.prompt.prompt_version,
        },
        registry: &plan.registry,
        anchor_allowlist: Some(ctx.allowlist),
    };
    plan.validator
        .validate_raw(cell.format, &raw, &context)
        .map_err(|errors| ctx.fail(Some(record_id), "self_validation", errors))?;
    Ok(ProducedDraft {
        record,
        raw,
        envelope: Some(envelope),
    })
}
