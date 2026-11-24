//! End-to-end tests of the admission pipeline (#9): the gold set admits
//! end to end, and every near-miss rejects at the stage
//! `data/gold/near-miss/manifest.json` names.
//!
//! Everything runs on scratch copies under the OS temp dir — the gold set
//! and `docs/sources.json` are never mutated.

mod common;

use std::fs;

use medsynth_validate::admit::Outcome;
use serde_json::Value;

use common::{Harness, Overrides, REVIEWER_RUN, gold_shard, outcome, sources_path, workspace_root};

#[test]
fn gold_set_is_admitted() {
    let mut harness = Harness::new("gold");
    for format in ["mcq", "case_vignette", "tutoring_example"] {
        harness.add_batch_file(&format!("{format}.jsonl"), &gold_shard(format));
    }
    harness.index();
    let opts = harness.opts(harness.config(true), &sources_path());
    let report = harness.settle(&opts, &Overrides::default());

    assert_eq!(report.counts.input, 24);
    assert_eq!(report.counts.admitted, 24);
    assert_eq!(report.counts.rejected, 0);
    assert_eq!(report.counts.held, 0);
    assert_eq!(report.admitted_shards.len(), 3);
    assert!(harness.dir.join("admission-log.json").exists());

    // The shard contents: admitted status and the run's contract version.
    let shard = fs::read_to_string(
        harness
            .dir
            .join("data/mcq")
            .join(format!("{}.jsonl", report.batch_id)),
    )
    .unwrap();
    for line in shard.lines() {
        let v: Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["verification_status"], "admitted");
        assert_eq!(v["contract_version"], "0.1");
    }
    // The review ran: a 5-record sample at rate 0.2 over 24 candidates.
    assert_eq!(report.review.sampled.len(), 5);
    assert!(
        report
            .review
            .results
            .iter()
            .all(|r| r.outcome == "pass" && r.reviewer_run_id == REVIEWER_RUN)
    );
}

#[test]
fn near_misses_reject_at_named_stages() {
    let mut harness = Harness::new("near-miss");
    let root = workspace_root().join("data/gold/near-miss");
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(root.join("manifest.json")).unwrap()).unwrap();
    let mut overrides = Overrides::default();
    for entry in manifest["records"].as_array().unwrap() {
        let file = entry["file"].as_str().unwrap();
        let text = fs::read_to_string(root.join(file)).unwrap();
        harness.add_batch_file(file, &text);
        let id = entry["id"].as_str().unwrap().to_owned();
        // Stage 2: the solver derives the manifest's expected answer,
        // which differs from the keyed one exactly on wrong-key records.
        if let Some(answer) = entry["expected_answer"].as_str() {
            overrides.solve_answer.insert(id.clone(), answer.to_owned());
        }
        // Stage 3: the defect manifest's semantic verdicts. A missing
        // anchor is an unanchored claim block; an unsupported claim cites
        // an anchor whose passage does not support it.
        match entry["defect"].as_str().unwrap() {
            "missing_anchor" => {
                overrides
                    .claim_verdict
                    .insert((id, "/stem".to_owned()), "unanchored".to_owned());
            }
            "unanchored_claim" => {
                overrides
                    .claim_verdict
                    .insert((id, "/tutor_response".to_owned()), "unsupported".to_owned());
            }
            _ => {}
        }
    }
    harness.index();
    let opts = harness.opts(harness.config(true), &sources_path());
    let report = harness.settle(&opts, &overrides);

    assert_eq!(report.counts.rejected, 6);
    assert_eq!(report.counts.admitted, 0);
    for entry in manifest["records"].as_array().unwrap() {
        let id = entry["id"].as_str().unwrap();
        let expected_stage = entry["verification_stage"].as_u64().unwrap() as u8;
        let o = outcome(&report, id);
        assert_eq!(
            o.outcome,
            Outcome::Rejected,
            "{id} should reject, got {:?}",
            o.outcome
        );
        assert_eq!(
            o.stage,
            Some(expected_stage),
            "{id} rejected at stage {:?}, manifest expects {expected_stage}",
            o.stage
        );
    }
}
