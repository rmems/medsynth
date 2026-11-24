//! Stage 2: a pinned solver re-derives the answer key from the blind
//! item (the record minus its key/rationale). The solver must not be the
//! model or run that drafted the record, and the check runs before any
//! request is emitted — a same-model solver never sees the item.

use serde_json::Value;

use crate::Format;

use super::batch::DraftRecord;
use super::claims;
use super::eval::{Ctx, Verdict, detail_suffix, identity_mismatch, respond, same_model};
use super::fulfill::RequestKind;

/// Re-derive the key and compare — or hold when the solver cannot answer.
pub(super) fn stage_key(ctx: &Ctx, record: &DraftRecord) -> Result<Verdict, String> {
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

    let body = claims::solve_body(&ctx.scope(), record, &item);
    let value = match respond(ctx, RequestKind::Solve, &body, "solver")? {
        Ok(value) => value,
        Err(reason) => return Ok(Verdict::hold(reason)),
    };
    if let Some(reason) =
        identity_mismatch(&value, &solver.run_id, Some(&solver.model_id), "solver")
    {
        return Ok(Verdict::hold(reason));
    }
    Ok(compare_key(record, &value))
}

/// Stage 2's verdict on a usable solver response.
fn compare_key(record: &DraftRecord, value: &Value) -> Verdict {
    match record.format {
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
        Format::CaseVignette => vignette_verdict(record, value),
        Format::TutoringExample => Verdict::Pass,
    }
}

/// Stage 2's verdict for a vignette: the solver must actually re-derive
/// the answers — a bare `agree` without derivation artifacts proves
/// nothing.
fn vignette_verdict(record: &DraftRecord, value: &Value) -> Verdict {
    let questions = record.value["questions"].as_array().map_or(0, Vec::len);
    let derived = value["answers"].as_array();
    if !derived.is_some_and(|a| a.len() == questions && a.iter().all(answer_present)) {
        return Verdict::hold(format!(
            "solver response carries no derived `answers` for the {questions} questions"
        ));
    }
    match value["verdict"].as_str() {
        Some("agree") => Verdict::Pass,
        Some("disagree") => Verdict::reject(format!(
            "solver disagrees with the expected answer{}",
            detail_suffix(value)
        )),
        _ => Verdict::hold("solver response carries no `verdict`"),
    }
}

/// A derived-answer entry that carries a non-blank string.
fn answer_present(x: &Value) -> bool {
    x.as_str().is_some_and(|s| !s.trim().is_empty())
}
