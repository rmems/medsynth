//! Per-record evaluation: the shared context, verdicts, and the ordered
//! stage dispatch.
//!
//! Stage verdicts are `Reject` (a genuine defect — the record never
//! reaches `data/`) or `Hold` (something that must be resolved before the
//! record can be judged — a stale registry hash, a missing fulfillment,
//! a failed review). Holds are reported separately from rejections so
//! infrastructure and provenance issues never masquerade as defects
//! (contract §8).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde_json::Value;

use super::batch::{Batch, DraftRecord};
use super::claims;
use super::config::RunConfig;
use super::fulfill::{Exchange, Fulfillment, RequestKind};
use super::outputs::display;
use super::pipeline::{Outcome, RecordOutcome, Stage};
use super::registry::SourceRegistry;
use super::teachers::TeacherPinRegistry;
use super::{anchor, intake, key};

/// Everything a record evaluation reads.
pub(super) struct Ctx<'a> {
    pub batch: &'a Batch,
    pub validators: &'a crate::Validators,
    pub registry: &'a SourceRegistry,
    pub teacher_registry: &'a TeacherPinRegistry,
    pub config: &'a RunConfig,
    pub fulfillment: &'a Fulfillment,
    /// Ids already admitted under `data/` (minus this run's own shards) —
    /// a re-admission is a stage-1 duplicate.
    pub admitted_ids: &'a HashSet<String>,
    /// How many batch records carry each id — stage 1's duplicate check
    /// without a per-record rescan.
    pub id_counts: &'a HashMap<String, usize>,
}

impl Ctx<'_> {
    /// The run identity request bodies are bound to.
    pub fn scope(&self) -> claims::RequestScope<'_> {
        claims::RequestScope {
            batch_id: &self.batch.batch_id,
            contract_version: &self.config.contract_version,
            registry_version: &self.registry.registry_version,
        }
    }
}

/// One stage's verdict on a record.
pub(super) enum Verdict {
    Pass,
    Reject(Vec<String>),
    Hold(Vec<String>),
}

impl Verdict {
    pub fn reject(reason: impl Into<String>) -> Self {
        Self::Reject(vec![reason.into()])
    }
    pub fn hold(reason: impl Into<String>) -> Self {
        Self::Hold(vec![reason.into()])
    }
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
}

/// Reject/hold reasons collected while a stage walks a record.
#[derive(Default)]
pub(super) struct Findings {
    rejects: Vec<String>,
    holds: Vec<String>,
}

impl Findings {
    pub fn reject(&mut self, reason: impl Into<String>) {
        self.rejects.push(reason.into());
    }
    pub fn hold(&mut self, reason: impl Into<String>) {
        self.holds.push(reason.into());
    }
    /// Rejects dominate holds; either dominates a pass.
    pub fn verdict(self) -> Verdict {
        if !self.rejects.is_empty() {
            Verdict::Reject(self.rejects)
        } else if !self.holds.is_empty() {
            Verdict::Hold(self.holds)
        } else {
            Verdict::Pass
        }
    }
}

/// How one record's evaluation concluded.
pub(super) struct RecordEval {
    pub outcome: Outcome,
    pub outcome_record: RecordOutcome,
    pub path: PathBuf,
    pub stage: Option<Stage>,
    /// Records that cleared stages 0–4 and entered the review pool.
    pub review_candidate: bool,
}

impl RecordEval {
    pub fn stage_label(&self) -> String {
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

/// Stages 0–4, in order — a record stops at its first non-pass verdict.
/// Cleared records come back as review candidates; the batch-level review
/// decides admitted vs held. Infallible by design: infrastructure
/// failures hold the record at the stage that hit them (contract §8),
/// they never sink the batch's run.
pub(super) fn evaluate(ctx: &Ctx, record: &DraftRecord) -> RecordEval {
    // Intake gates schema: a draft that fails the envelope checks never
    // incurs the schema scan.
    let v = intake::stage_intake(ctx, record);
    if !v.is_pass() {
        return stopped(record, v, Stage::Intake, "unverified");
    }
    let v = intake::stage_schema(ctx, record);
    if !v.is_pass() {
        return stopped(record, v, Stage::Schema, "unverified");
    }
    evaluate_key(ctx, record)
}

fn evaluate_key(ctx: &Ctx, record: &DraftRecord) -> RecordEval {
    let v = key::stage_key(ctx, record)
        .unwrap_or_else(|e| Verdict::hold(format!("stage-2 evaluation error: {e}")));
    if !v.is_pass() {
        return stopped(record, v, Stage::Key, "unverified");
    }
    evaluate_anchor(ctx, record)
}

fn evaluate_anchor(ctx: &Ctx, record: &DraftRecord) -> RecordEval {
    let result = anchor::stage_anchor(ctx, record);
    let (v, verify_response) = match result {
        Ok(pair) => pair,
        Err(e) => {
            let v = Verdict::hold(format!("stage-3 evaluation error: {e}"));
            return stopped(record, v, Stage::Anchor, "key-checked");
        }
    };
    if !v.is_pass() {
        return stopped(record, v, Stage::Anchor, "key-checked");
    }
    let v = anchor::stage_distractor(record, verify_response.as_ref());
    if !v.is_pass() {
        return stopped(record, v, Stage::Distractor, "anchor-checked");
    }
    RecordEval {
        outcome: Outcome::Admitted,
        outcome_record: RecordOutcome {
            record_id: record.record_id(),
            format: record.format.slug().to_owned(),
            path: display(&record.path),
            outcome: Outcome::Admitted,
            stage: None,
            stage_name: None,
            status: "anchor-checked".to_owned(),
            reasons: Vec::new(),
        },
        path: record.path.clone(),
        stage: None,
        review_candidate: true,
    }
}

// ---------------------------------------------------------------------------
// Shared response helpers — the three semantic stages speak the same
// recorded-fulfillment protocol.
// ---------------------------------------------------------------------------

/// One fulfillment round-trip: `Ok(value)` on a usable response,
/// `Err(reason)` for the hold an absent or malformed response earns.
/// `Err` at the top level is an operational I/O failure.
pub(super) fn respond(
    ctx: &Ctx,
    kind: RequestKind,
    body: &Value,
    label: &str,
) -> Result<Result<Value, String>, String> {
    Ok(match ctx.fulfillment.exchange(kind, body) {
        Ok(Exchange::Response { value }) => Ok(value),
        Ok(Exchange::Unavailable { key }) => {
            Err(format!("{label} fulfillment unavailable ({key})"))
        }
        Ok(Exchange::Malformed { key, detail }) => Err(format!(
            "{label} response {key} is not usable JSON: {detail}"
        )),
        Err(e) => return Err(format!("{label} fulfillment I/O: {e}")),
    })
}

/// Whether `model_id`/`pin_id` collides with the drafting identity: the
/// rights envelope's model, served model, or pin, or `provenance.model`.
pub(super) fn same_model(
    ctx: &Ctx,
    record: &DraftRecord,
    model_id: &str,
    pin_id: Option<&str>,
) -> bool {
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
/// pinned). A mismatch earns a hold reason — the harness answered a
/// different request than the gate sent.
pub(super) fn identity_mismatch(
    response: &Value,
    run_id: &str,
    model_id: Option<&str>,
    label: &str,
) -> Option<String> {
    if response["run_id"].as_str() != Some(run_id) {
        return Some(format!(
            "{label} response names run `{}`, not the pinned `{run_id}`",
            response["run_id"].as_str().unwrap_or("<missing>")
        ));
    }
    // A pinned model must be confirmed, not merely uncontradicted — a
    // response that omits model_id could come from any model.
    if let Some(model) = model_id
        && response["model_id"].as_str() != Some(model)
    {
        return Some(format!(
            "{label} response names model `{}`, not the pinned `{model}`",
            response["model_id"].as_str().unwrap_or("<missing>")
        ));
    }
    None
}

/// Whether a fulfillment response names the drafting model — a
/// correctly pinned run answering with the drafting model is still the
/// drafting model. Runs after identity confirmation.
pub(super) fn response_is_drafter(
    ctx: &Ctx,
    record: &DraftRecord,
    response: &Value,
    label: &str,
) -> Option<String> {
    let model = response["model_id"].as_str()?;
    same_model(ctx, record, model, None).then(|| {
        format!(
            "{label} response names the drafting model `{model}` — the check is not independent"
        )
    })
}

/// The other independence failure mode: when the pin carries no
/// `model_id` of its own, a response naming no model at all leaves
/// independence unconfirmable — the drafting model could be fulfilling
/// its own check by simply not naming itself. When the pin does carry a
/// model, identity confirmation already forces the response to name it,
/// so the gap cannot occur.
pub(super) fn unconfirmable_independence(
    response: &Value,
    pin_model: Option<&str>,
    label: &str,
) -> Option<String> {
    (pin_model.is_none() && response["model_id"].as_str().is_none()).then(|| {
        format!("{label} response carries no `model_id` — independence cannot be confirmed")
    })
}

/// `": <detail>"` when a response carries a detail string, else empty —
/// appended to reject/hold reasons.
pub(super) fn detail_suffix(response: &Value) -> String {
    response["detail"]
        .as_str()
        .map(|d| format!(": {d}"))
        .unwrap_or_default()
}
