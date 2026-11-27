//! Case-vignette solver comparison tests (#9).

mod common;

use common::{Harness, Overrides, gold_shard, outcome, sources_path};
use medsynth_validate::admit::Outcome;
use serde_json::Value;

#[test]
fn vignette_answer_mismatch_rejects_at_stage_2() {
    // The solver re-derives the vignette answers and the gate compares
    // them against the record — agreement in verdict alone proves nothing.
    let mut harness = Harness::new("vignette-mismatch");
    let record: Value =
        serde_json::from_str(gold_shard("case_vignette").lines().next().unwrap()).unwrap();
    let id = record["id"].as_str().unwrap().to_owned();
    harness.add_batch_file(
        "case_vignette.jsonl",
        &serde_json::to_string(&record).unwrap(),
    );
    harness.index();
    let wrong: Vec<String> = record["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|_| "an entirely unrelated derivation".to_owned())
        .collect();
    let overrides = Overrides {
        solve_vignette_answers: [(id.clone(), wrong)].into_iter().collect(),
        ..Overrides::default()
    };
    let opts = harness.opts(harness.config(true), &sources_path());
    let report = harness.settle(&opts, &overrides);

    let o = outcome(&report, &id);
    assert_eq!(o.outcome, Outcome::Rejected);
    assert_eq!(o.stage, Some(2));
    assert!(
        o.reasons.iter().any(|r| r.contains("solver derived")),
        "missing comparison reason: {:?}",
        o.reasons
    );
}

#[test]
fn vignette_answer_restatement_passes() {
    // A correct derivation worded differently still agrees: normalized
    // containment is enough. The record's answer restated with the
    // conclusion leading must pass.
    let mut harness = Harness::new("vignette-restate");
    let record: Value =
        serde_json::from_str(gold_shard("case_vignette").lines().next().unwrap()).unwrap();
    let id = record["id"].as_str().unwrap().to_owned();
    harness.add_batch_file(
        "case_vignette.jsonl",
        &serde_json::to_string(&record).unwrap(),
    );
    harness.index();
    let restated: Vec<String> = record["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|q| {
            let text = q["answer"]["text"].as_str().unwrap();
            format!("The answer is: {text}")
        })
        .collect();
    let overrides = Overrides {
        solve_vignette_answers: [(id.clone(), restated)].into_iter().collect(),
        ..Overrides::default()
    };
    let opts = harness.opts(harness.config(true), &sources_path());
    let report = harness.settle(&opts, &overrides);

    let o = outcome(&report, &id);
    assert_eq!(
        o.outcome,
        Outcome::Admitted,
        "restated correct answers must pass: {:?}",
        o.reasons
    );
}
