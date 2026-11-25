//! Stage 5: an agent-driven accuracy review over a deterministic sample
//! of the cleared records. The batch rule: a sample that does not pass
//! holds the whole batch — unaudited records are suspect too, so nothing
//! is admitted.

use std::collections::HashMap;

use super::batch::DraftRecord;
use super::claims;
use super::config::AgentPin;
use super::eval::{Ctx, RecordEval, identity_mismatch, respond, response_is_drafter, same_model};
use super::fulfill::RequestKind;
use super::pipeline::{Outcome, ReviewReport, ReviewResult, Stage};

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

/// Run the review sample over the cleared records and settle the batch:
/// every sampled record passes → all candidates are admitted; any
/// failure holds every candidate.
pub(super) fn run_review(ctx: &Ctx, results: &mut [RecordEval]) -> Result<ReviewReport, String> {
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

    for &i in &sampled {
        let record = ctx
            .batch
            .records
            .iter()
            .find(|r| {
                r.record_id() == results[i].outcome_record.record_id && r.path == results[i].path
            })
            .expect("outcome records come from the batch");
        report.results.push(review_one(ctx, record, &reviewer)?);
    }

    if report.results.iter().all(|r| r.outcome == "pass") {
        for i in candidates {
            results[i].outcome_record.status = "admitted".to_owned();
        }
    } else {
        hold_batch(&mut report, results, &candidates);
    }
    Ok(report)
}

/// One sampled record's review: the same-model guard, the fulfillment
/// exchange, identity pinning, then the recorded outcome.
fn review_one(
    ctx: &Ctx,
    record: &DraftRecord,
    reviewer: &AgentPin,
) -> Result<ReviewResult, String> {
    let result = |outcome: &str, detail: Option<String>| ReviewResult {
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
        return Ok(result(
            "held",
            Some("reviewer is the drafting model".to_owned()),
        ));
    }
    let body = claims::review_body(&ctx.scope(), record);
    let outcome = match respond(ctx, RequestKind::Review, &body, "review")? {
        Ok(value) => value,
        Err(reason) => return Ok(result("unfulfilled", Some(reason))),
    };
    if let Some(reason) = identity_mismatch(
        &outcome,
        &reviewer.run_id,
        reviewer.model_id.as_deref(),
        "reviewer",
    ) {
        return Ok(result("identity-mismatch", Some(reason)));
    }
    // A pin without a model_id cannot pre-check independence — but the
    // response names its model, and the drafting model reviewing its own
    // record is not a review.
    if let Some(reason) = response_is_drafter(ctx, record, &outcome, "reviewer") {
        return Ok(result("held", Some(reason)));
    }
    match outcome["outcome"].as_str() {
        Some(name) => Ok(result(name, outcome["detail"].as_str().map(str::to_owned))),
        None => Ok(result(
            "no-outcome",
            Some("review response carries no `outcome`".to_owned()),
        )),
    }
}

/// Hold every review candidate: per-record reasons for the sampled
/// failures, a batch-level reason for the rest.
fn hold_batch(report: &mut ReviewReport, results: &mut [RecordEval], candidates: &[usize]) {
    // Only failed results carry per-record reasons — a sampled record
    // that passed but lost the batch lottery gets the generic hold.
    let reasons: HashMap<String, String> = report
        .results
        .iter()
        .filter(|r| r.outcome != "pass")
        .map(|r| {
            (
                r.record_id.clone(),
                format!("{}: {}", r.outcome, r.detail.clone().unwrap_or_default()),
            )
        })
        .collect();
    for &i in candidates {
        let id = &results[i].outcome_record.record_id;
        let reason = reasons
            .get(id)
            .cloned()
            .unwrap_or_else(|| "batch held: the sampled review did not pass".to_owned());
        hold(&mut results[i], &reason);
    }
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
