//! Model-independence tests across teacher drafting, solving, and verification (#9).

mod common;

use common::teacher::{
    DraftCase, SAME_MODEL_CASE, settle_prepared_fixture, setup_teacher_case, teacher_draft,
    write_teacher_manifest,
};
use common::{Harness, Overrides, VERIFIER_MODEL, gold_shard, outcome, sources_path};
use medsynth_validate::admit::Outcome;
use serde_json::Value;

#[test]
fn same_model_draft_rejects_without_solving() {
    // A teacher-lane draft whose rights envelope names the stage-2 solver
    // as its drafting model must be refused before a solve request is
    // ever emitted.
    let fixture = setup_teacher_case(SAME_MODEL_CASE);
    write_teacher_manifest(&fixture);
    let (fixture, report) = settle_prepared_fixture(fixture);

    let o = outcome(&report, fixture.case.draft.id);
    assert_eq!((o.outcome, o.stage), (Outcome::Rejected, Some(2)));
    // The check ran before any request was written.
    assert_eq!(fixture.harness.request_count("solve"), 0);
}

#[test]
fn verifier_same_model_holds_before_verifying() {
    // A verifier pinned to the drafting model never sees the claims.
    let mut harness = Harness::new("verifier-same-model");
    let mut record: Value =
        serde_json::from_str(gold_shard("mcq").lines().next().unwrap()).unwrap();
    let id = "verifier-same-model-0001";
    teacher_draft(
        &harness,
        &DraftCase {
            id,
            model: VERIFIER_MODEL,
        },
        &mut record,
    );
    harness.index();
    let opts = harness.opts(harness.config(true), &sources_path());
    let report = harness.settle(&opts, &Overrides::default());

    let o = outcome(&report, id);
    assert_eq!((o.outcome, o.stage), (Outcome::Held, Some(3)));
    assert!(
        o.reasons
            .iter()
            .any(|r| r.contains("verifier is the model that drafted")),
        "missing independence hold: {:?}",
        o.reasons
    );
    assert_eq!(harness.request_count("verify"), 0);
}

#[test]
fn verifier_response_from_drafting_model_holds() {
    // A pin without a model_id cannot pre-check independence — the
    // response's own model is compared instead.
    let mut harness = Harness::new("verifier-unpinned-model");
    let mut record: Value =
        serde_json::from_str(gold_shard("mcq").lines().next().unwrap()).unwrap();
    let id = "verifier-unpinned-0001";
    teacher_draft(
        &harness,
        &DraftCase {
            id,
            model: VERIFIER_MODEL,
        },
        &mut record,
    );
    harness.index();
    let mut config = harness.config(true);
    config.verifier.as_mut().unwrap().model_id = None;
    let opts = harness.opts(config, &sources_path());
    let report = harness.settle(&opts, &Overrides::default());

    let o = outcome(&report, id);
    assert_eq!((o.outcome, o.stage), (Outcome::Held, Some(3)));
    assert!(
        o.reasons.iter().any(|r| r.contains("not independent")),
        "missing response-model hold: {:?}",
        o.reasons
    );
}
