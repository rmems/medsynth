//! Gate-path tests for the admission pipeline (#9): a same-model solver
//! never sees a request, an unpinned solver refuses to start, a stale
//! registry entry holds for re-anchoring, a failed distractor screen
//! rejects at stage 4, and a schema defect rejects at stage 1.
//!
//! Everything runs on scratch copies under the OS temp dir — the gold set
//! and `docs/sources.json` are never mutated.

mod common;

use std::fs;

use medsynth_validate::admit::{self, Outcome};
use serde_json::{Value, json};

use common::{
    Harness, Overrides, SOLVER_MODEL, gold_shard, outcome, sha256_hex, sources_path, workspace_root,
};

#[test]
fn same_model_draft_rejects_without_solving() {
    // A teacher-lane draft whose rights envelope names the stage-2 solver
    // as its drafting model must be refused before a solve request is
    // ever emitted.
    let mut harness = Harness::new("same-model");
    let mut record: Value =
        serde_json::from_str(gold_shard("mcq").lines().next().unwrap()).unwrap();
    record["id"] = json!("same-model-draft-0001");
    let id = "same-model-draft-0001";
    record["provenance"]["model"] = json!(SOLVER_MODEL);
    record["provenance"]["prompt_version"] = json!("v1");
    let text = serde_json::to_string(&record).unwrap();
    let content_sha = sha256_hex(text.as_bytes());
    harness.add_batch_file("mcq/same-model-0001.json", &text);
    let pin_entry = "test teacher entry";
    fs::write(
        harness.dir.join("batch/manifest.json"),
        serde_json::to_string_pretty(&json!({
            "manifest_version": 1,
            "batch_id": "same-model-batch",
            "records": [{"id": id, "lane": "teacher", "sha256": content_sha}],
            "rights_envelopes": [{
                "record_id": id,
                "teacher_variant": SOLVER_MODEL,
                "model_id": SOLVER_MODEL,
                "pin_id": "pin-test",
                "pin_entry": pin_entry,
                "pin_entry_sha256": sha256_hex(pin_entry.as_bytes()),
                "content_sha256": content_sha,
            }],
        }))
        .unwrap(),
    )
    .unwrap();
    harness.index();
    let opts = harness.opts(harness.config(true), &sources_path());
    let report = harness.settle(&opts, &Overrides::default());

    let o = outcome(&report, id);
    assert_eq!(o.outcome, Outcome::Rejected);
    assert_eq!(o.stage, Some(2));
    // The check ran before any request was written.
    assert_eq!(harness.request_count("solve"), 0);
}

#[test]
fn unpinned_solver_refuses_to_start() {
    let mut harness = Harness::new("unpinned");
    harness.add_batch_file("mcq.jsonl", &gold_shard("mcq"));
    harness.index();
    // Same shape the loader rejects: no solver pin over keyed records.
    let opts = harness.opts(harness.config(false), &sources_path());
    let err = admit::run(&opts).expect_err("unpinned solver must refuse");
    assert!(err.contains("solver"), "unexpected error: {err}");
    // And the whole batch produced no output.
    assert!(!harness.dir.join("admission-log.json").exists());
}

#[test]
fn stale_registry_entry_holds_for_reanchoring() {
    // A registry entry that changed since drafting holds its records —
    // never a rejection, per contract §8/§7.
    let mut harness = Harness::new("stale-registry");
    let batch_src = workspace_root().join("drafts/sample-batch");
    for entry in fs::read_dir(&batch_src).unwrap().flatten() {
        let path = entry.path();
        if path.is_file()
            && matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("json") | Some("jsonl")
            )
        {
            fs::copy(&path, harness.dir.join("batch").join(entry.file_name())).unwrap();
        }
    }
    harness.index();
    // Drift every registry entry: the receipts' pinned hashes no longer
    // match anything current.
    let mut sources: Value =
        serde_json::from_str(&fs::read_to_string(sources_path()).unwrap()).unwrap();
    for entry in sources["sources"].as_array_mut().unwrap() {
        entry["year"] = json!(1900);
    }
    let drifted = harness.dir.join("sources-drifted.json");
    fs::write(&drifted, serde_json::to_string_pretty(&sources).unwrap()).unwrap();

    let opts = harness.opts(harness.config(true), &drifted);
    let report = harness.settle(&opts, &Overrides::default());

    assert_eq!(report.counts.rejected, 0);
    assert_eq!(report.counts.admitted, 0);
    assert_eq!(report.counts.held, report.counts.input);
    assert!(report.held_by_stage.contains_key("3-anchor"));
    for o in &report.outcomes {
        assert_eq!(o.stage, Some(3), "{} held at {:?}", o.record_id, o.stage);
        assert!(
            o.reasons
                .iter()
                .any(|r| r.contains("changed since drafting")),
            "{} missing stale-registry hold: {:?}",
            o.record_id,
            o.reasons
        );
    }
}

#[test]
fn failed_distractor_screen_rejects_at_stage_4() {
    let mut harness = Harness::new("distractor");
    harness.add_batch_file("mcq.jsonl", &gold_shard("mcq"));
    harness.index();
    let overrides = Overrides {
        distractor: Some("fail".to_owned()),
        ..Overrides::default()
    };
    let opts = harness.opts(harness.config(true), &sources_path());
    let report = harness.settle(&opts, &overrides);

    assert_eq!(report.counts.rejected, report.counts.input);
    for o in &report.outcomes {
        assert_eq!(
            o.stage,
            Some(4),
            "{} rejected at {:?}",
            o.record_id,
            o.stage
        );
        assert_eq!(o.status, "anchor-checked");
    }
}

#[test]
fn schema_defect_rejects_at_stage_1() {
    let mut harness = Harness::new("schema");
    let mut record: Value =
        serde_json::from_str(gold_shard("mcq").lines().next().unwrap()).unwrap();
    record["key"] = json!("Z"); // not a valid option label
    harness.add_batch_file(
        "mcq/bad-0001.json",
        &serde_json::to_string(&record).unwrap(),
    );
    harness.index();
    let opts = harness.opts(harness.config(true), &sources_path());
    let report = harness.settle(&opts, &Overrides::default());

    assert_eq!(report.counts.rejected, 1);
    let o = &report.outcomes[0];
    assert_eq!(o.outcome, Outcome::Rejected);
    assert_eq!(o.stage, Some(1));
    assert_eq!(harness.request_count("solve"), 0);
}
