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
    Harness, Overrides, SOLVER_MODEL, VERIFIER_MODEL, gold_shard, outcome, sha256_hex,
    sources_path, workspace_root,
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

/// `load_run_config` parses a good file and names the reason each
/// malformed variant fails: unreadable, unparseable, blank or stale
/// contract version, out-of-range sample rate, incomplete pins, and
/// colliding run ids.
#[test]
fn run_config_load_validates_shape() {
    let harness = Harness::new("config-shape");
    let path = harness.dir.join("run-config.json");
    let write = |body: &str| {
        fs::write(&path, body).unwrap();
        admit::load_run_config(&path)
    };
    let good = json!({
        "fulfillment_dir": "fulfill",
        "contract_version": "0.1",
        "solver": {"run_id": "s1", "model_id": "m1"},
        "verifier": {"run_id": "v1"},
        "reviewer": {"run_id": "r1"},
    });
    let config = write(&good.to_string()).expect("valid config");
    assert_eq!(config.contract_version, "0.1");
    assert_eq!(config.sample_rate, 0.10); // default

    // Unreadable vs unparseable are distinct diagnostics.
    let missing = admit::load_run_config(&harness.dir.join("no-such-config.json"));
    assert!(missing.unwrap_err().contains("cannot read"));
    assert!(write("{nope").unwrap_err().contains("run config"));

    let mut v = good.clone();
    // Blank, padded, and stale versions all fail the exact-match check —
    // the string is stamped verbatim, so no trimming tolerance.
    for bad in [" ", " 0.1 ", "0.0"] {
        v["contract_version"] = json!(bad);
        assert!(
            write(&v.to_string())
                .unwrap_err()
                .contains("not the gate's"),
            "contract_version `{bad}` must fail exact match"
        );
    }
    v = good.clone();
    v["sample_rate"] = json!(1.5);
    assert!(write(&v.to_string()).unwrap_err().contains("sample_rate"));

    v = good.clone();
    v["solver"] = json!({"run_id": " ", "model_id": "m"});
    assert!(write(&v.to_string()).unwrap_err().contains("solver pin"));

    v = good.clone();
    v["verifier"] = json!({"run_id": " "});
    assert!(write(&v.to_string()).unwrap_err().contains("verifier"));

    // A present-but-blank optional model id is a malformed pin, not an
    // unpinned model.
    v = good.clone();
    v["verifier"] = json!({"run_id": "v1", "model_id": "  "});
    assert!(write(&v.to_string()).unwrap_err().contains("model_id"));
    v = good.clone();
    v["reviewer"] = json!({"run_id": "r1", "model_id": "	"});
    assert!(write(&v.to_string()).unwrap_err().contains("model_id"));

    v = good.clone();
    v["reviewer"] = json!({"run_id": "s1"});
    assert!(write(&v.to_string()).unwrap_err().contains("distinct"));
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

/// A teacher-lane draft whose envelope names `model` as its drafting
/// model — the same-model test pattern, parameterized by the model.
fn teacher_draft(harness: &Harness, id: &str, record: &mut Value, model: &str) {
    record["id"] = json!(id);
    record["provenance"]["model"] = json!(model);
    record["provenance"]["prompt_version"] = json!("v1");
    let text = serde_json::to_string(&record).unwrap();
    let content_sha = sha256_hex(text.as_bytes());
    harness.add_batch_file(&format!("mcq/{id}.json"), &text);
    let pin_entry = "test teacher entry";
    fs::write(
        harness.dir.join("batch/manifest.json"),
        serde_json::to_string_pretty(&json!({
            "manifest_version": 1,
            "batch_id": format!("{id}-batch"),
            "records": [{"id": id, "lane": "teacher", "sha256": content_sha}],
            "rights_envelopes": [{
                "record_id": id,
                "teacher_variant": model,
                "model_id": model,
                "pin_id": "pin-test",
                "pin_entry": pin_entry,
                "pin_entry_sha256": sha256_hex(pin_entry.as_bytes()),
                "content_sha256": content_sha,
            }],
        }))
        .unwrap(),
    )
    .unwrap();
}

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

#[test]
fn verifier_same_model_holds_before_verifying() {
    // A verifier pinned to the drafting model never sees the claims.
    let mut harness = Harness::new("verifier-same-model");
    let mut record: Value =
        serde_json::from_str(gold_shard("mcq").lines().next().unwrap()).unwrap();
    let id = "verifier-same-model-0001";
    teacher_draft(&harness, id, &mut record, VERIFIER_MODEL);
    harness.index();
    let opts = harness.opts(harness.config(true), &sources_path());
    let report = harness.settle(&opts, &Overrides::default());

    let o = outcome(&report, id);
    assert_eq!(o.outcome, Outcome::Held);
    assert_eq!(o.stage, Some(3));
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
    teacher_draft(&harness, id, &mut record, VERIFIER_MODEL);
    harness.index();
    let mut config = harness.config(true);
    config.verifier.as_mut().unwrap().model_id = None;
    let opts = harness.opts(config, &sources_path());
    let report = harness.settle(&opts, &Overrides::default());

    let o = outcome(&report, id);
    assert_eq!(o.outcome, Outcome::Held);
    assert_eq!(o.stage, Some(3));
    assert!(
        o.reasons.iter().any(|r| r.contains("not independent")),
        "missing response-model hold: {:?}",
        o.reasons
    );
}
