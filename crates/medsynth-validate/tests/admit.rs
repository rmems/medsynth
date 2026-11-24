//! End-to-end tests of the admission pipeline (#9): the gold set admits,
//! every near-miss rejects at the stage `data/gold/near-miss/manifest.json`
//! names, a same-model solver never sees a request, an unpinned solver
//! refuses to start, a stale registry entry holds for re-anchoring, and a
//! failed distractor screen rejects at stage 4.
//!
//! Everything runs on scratch copies under the OS temp dir — the gold set
//! and `docs/sources.json` are never mutated.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use medsynth_validate::admit::{
    self, AdmissionOptions, AdmissionReport, AgentPin, Outcome, RunConfig, SolverPin,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const SOLVER_RUN: &str = "solver-run-test";
const SOLVER_MODEL: &str = "solver-model-test";
const VERIFIER_RUN: &str = "verifier-run-test";
const VERIFIER_MODEL: &str = "verifier-model-test";
const REVIEWER_RUN: &str = "reviewer-run-test";
const REVIEWER_MODEL: &str = "reviewer-model-test";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .to_path_buf()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Per-record overrides on the default responder.
#[derive(Default)]
struct Overrides {
    /// record id -> the answer the solver claims it derived.
    solve_answer: HashMap<String, String>,
    /// (record id, claim path) -> the verifier's verdict for that block.
    claim_verdict: HashMap<(String, String), String>,
    /// The distractor screen's verdict (`pass` unless overridden).
    distractor: Option<String>,
    /// The review outcome (`pass` unless overridden).
    review_outcome: Option<String>,
}

/// The default fulfiller: solver answers the record's key (or the
/// override), every claim is `supported` when anchored and `no-claim` when
/// not (or the override), distractor and review pass.
fn response(records: &HashMap<String, Value>, body: &Value, overrides: &Overrides) -> Value {
    let rid = body["record_id"].as_str().unwrap_or("").to_owned();
    match body["kind"].as_str() {
        Some("solve") => {
            let mut resp = json!({"run_id": SOLVER_RUN, "model_id": SOLVER_MODEL});
            match body["format"].as_str() {
                Some("mcq") => {
                    let answer = overrides
                        .solve_answer
                        .get(&rid)
                        .cloned()
                        .unwrap_or_else(|| records[&rid]["key"].as_str().unwrap().to_owned());
                    resp["answer"] = json!(answer);
                }
                Some("case_vignette") => resp["verdict"] = json!("agree"),
                _ => {}
            }
            resp
        }
        Some("verify") => {
            let claims: Vec<Value> = body["claims"]
                .as_array()
                .map(|claims| {
                    claims
                        .iter()
                        .map(|c| {
                            let path = c["path"].as_str().unwrap().to_owned();
                            let verdict = overrides
                                .claim_verdict
                                .get(&(rid.clone(), path.clone()))
                                .cloned()
                                .unwrap_or_else(|| {
                                    if c["anchors"].as_array().is_some_and(|a| !a.is_empty()) {
                                        "supported".to_owned()
                                    } else {
                                        "no-claim".to_owned()
                                    }
                                });
                            json!({"path": path, "verdict": verdict})
                        })
                        .collect()
                })
                .unwrap_or_default();
            json!({
                "run_id": VERIFIER_RUN,
                "model_id": VERIFIER_MODEL,
                "claims": claims,
                "distractor": {"verdict": overrides.distractor.clone().unwrap_or_else(|| "pass".to_owned())},
            })
        }
        Some("review") => json!({
            "run_id": REVIEWER_RUN,
            "model_id": REVIEWER_MODEL,
            "outcome": overrides.review_outcome.clone().unwrap_or_else(|| "pass".to_owned()),
        }),
        _ => json!({}),
    }
}

/// A scratch batch run: batch dir, fulfillment dir, data dir, log path.
struct Harness {
    dir: PathBuf,
    records: HashMap<String, Value>,
}

impl Harness {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("medsynth-admit-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("batch"))
            .and_then(|()| fs::create_dir_all(dir.join("fulfill")))
            .and_then(|()| fs::create_dir_all(dir.join("data")))
            .unwrap();
        Self {
            dir,
            records: HashMap::new(),
        }
    }

    /// Copy a record-bearing file into the batch (json shard or single
    /// json record) and index the records it carries.
    fn add_batch_file(&self, rel: &str, text: &str) -> Vec<String> {
        let path = self.dir.join("batch").join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        let mut ids = Vec::new();
        if rel.ends_with(".jsonl") {
            for line in text.lines().filter(|l| !l.trim().is_empty()) {
                let v: Value = serde_json::from_str(line).unwrap();
                ids.push(v["id"].as_str().unwrap().to_owned());
            }
        } else {
            let v: Value = serde_json::from_str(text).unwrap();
            ids.push(v["id"].as_str().unwrap().to_owned());
        }
        ids
    }

    fn index(&mut self) -> &mut Self {
        for entry in walk_json(&self.dir.join("batch")) {
            let text = fs::read_to_string(&entry).unwrap();
            if entry.extension().is_some_and(|e| e == "jsonl") {
                for line in text.lines().filter(|l| !l.trim().is_empty()) {
                    let v: Value = serde_json::from_str(line).unwrap();
                    self.records.insert(v["id"].as_str().unwrap().to_owned(), v);
                }
            } else if entry.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                n != "manifest.json" && n != "config.json" && !n.ends_with("-manifest.json")
            }) {
                let v: Value = serde_json::from_str(&text).unwrap();
                self.records.insert(v["id"].as_str().unwrap().to_owned(), v);
            }
        }
        self
    }

    fn config(&self, solver: bool) -> RunConfig {
        RunConfig {
            fulfillment_dir: self.dir.join("fulfill"),
            solver: solver.then(|| SolverPin {
                run_id: SOLVER_RUN.to_owned(),
                model_id: SOLVER_MODEL.to_owned(),
                pin_id: None,
            }),
            verifier: Some(AgentPin {
                run_id: VERIFIER_RUN.to_owned(),
                model_id: Some(VERIFIER_MODEL.to_owned()),
            }),
            reviewer: Some(AgentPin {
                run_id: REVIEWER_RUN.to_owned(),
                model_id: Some(REVIEWER_MODEL.to_owned()),
            }),
            sample_rate: 0.2,
            contract_version: "0.1".to_owned(),
        }
    }

    fn opts(&self, config: RunConfig, sources: &Path) -> AdmissionOptions {
        AdmissionOptions {
            batch_dir: self.dir.join("batch"),
            sources_path: sources.to_path_buf(),
            data_dir: self.dir.join("data"),
            log_path: self.dir.join("admission-log.json"),
            config,
        }
    }

    /// Run the pipeline, answering any unanswered request files after each
    /// pass, until a pass produces no new requests — the same loop an
    /// external fulfiller drives.
    fn settle(&self, opts: &AdmissionOptions, overrides: &Overrides) -> AdmissionReport {
        let mut report = admit::run(opts).expect("pipeline run");
        for _ in 0..8 {
            let mut pending = 0;
            let fulfill = &opts.config.fulfillment_dir;
            for entry in fs::read_dir(fulfill).unwrap().flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.ends_with(".request.json") {
                    continue;
                }
                let response_path = entry
                    .path()
                    .with_file_name(name.replace(".request.json", ".response.json"));
                if response_path.exists() {
                    continue;
                }
                let body: Value =
                    serde_json::from_str(&fs::read_to_string(entry.path()).unwrap()).unwrap();
                let resp = response(&self.records, &body, overrides);
                fs::write(&response_path, serde_json::to_string_pretty(&resp).unwrap()).unwrap();
                pending += 1;
            }
            if pending == 0 {
                return report;
            }
            report = admit::run(opts).expect("pipeline run");
        }
        report
    }

    /// Count request files of one kind in the fulfillment dir.
    fn request_count(&self, kind: &str) -> usize {
        fs::read_dir(self.dir.join("fulfill"))
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .ends_with(&format!(".{kind}.request.json"))
            })
            .count()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn walk_json(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk_json(&path));
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("json") | Some("jsonl")
        ) {
            out.push(path);
        }
    }
    out
}

fn gold_shard(format: &str) -> String {
    fs::read_to_string(
        workspace_root()
            .join("data/gold")
            .join(format!("{format}.jsonl")),
    )
    .unwrap()
}

fn sources_path() -> PathBuf {
    workspace_root().join("docs/sources.json")
}

fn outcome<'a>(
    report: &'a AdmissionReport,
    id: &str,
) -> &'a medsynth_validate::admit::RecordOutcome {
    report
        .outcomes
        .iter()
        .find(|o| o.record_id == id)
        .unwrap_or_else(|| panic!("no outcome for {id}"))
}

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
    fs::write(
        harness.dir.join("batch/manifest.json"),
        serde_json::to_string_pretty(&json!({
            "manifest_version": 1,
            "batch_id": "same-model-batch",
            "records": [{"id": id, "lane": "teacher"}],
            "rights_envelopes": [{
                "record_id": id,
                "teacher_variant": SOLVER_MODEL,
                "model_id": SOLVER_MODEL,
                "pin_id": "pin-test",
                "pin_entry": "test teacher entry",
                "pin_entry_sha256": "abc123",
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
