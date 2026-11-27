//! Shared harness for the admission-pipeline tests: scratch batch runs,
//! a scripted fulfiller that answers the recorded request files, and
//! report lookups. Items are shared across test targets, so dead-code
//! allowances apply per target.
#![allow(dead_code)]

pub mod teacher;

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use medsynth_validate::admit::{
    self, AdmissionOptions, AdmissionReport, AgentPin, RunConfig, SolverPin,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const SOLVER_RUN: &str = "solver-run-test";
pub const SOLVER_MODEL: &str = "solver-model-test";
pub const VERIFIER_RUN: &str = "verifier-run-test";
pub const VERIFIER_MODEL: &str = "verifier-model-test";
pub const REVIEWER_RUN: &str = "reviewer-run-test";
pub const REVIEWER_MODEL: &str = "reviewer-model-test";

pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .to_path_buf()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Per-record overrides on the default responder.
#[derive(Default)]
pub struct Overrides {
    /// record id -> the answer the solver claims it derived.
    pub solve_answer: HashMap<String, String>,
    /// record id -> vignette answers the solver claims it derived,
    /// replacing the faithful echo of the record's answers.
    pub solve_vignette_answers: HashMap<String, Vec<String>>,
    /// (record id, claim path) -> the verifier's verdict for that block.
    pub claim_verdict: HashMap<(String, String), String>,
    /// The distractor screen's verdict (`pass` unless overridden).
    pub distractor: Option<String>,
    /// The review outcome (`pass` unless overridden).
    pub review_outcome: Option<String>,
}

/// The solve response: the record's key (or the override) for MCQs, a
/// faithful echo of the vignette answers (or the override) plus `agree`.
fn solve_response(
    records: &HashMap<String, Value>,
    body: &Value,
    overrides: &Overrides,
    rid: &str,
) -> Value {
    let mut resp = json!({"run_id": SOLVER_RUN, "model_id": SOLVER_MODEL});
    match body["format"].as_str() {
        Some("mcq") => {
            let answer = overrides
                .solve_answer
                .get(rid)
                .cloned()
                .unwrap_or_else(|| records[rid]["key"].as_str().unwrap().to_owned());
            resp["answer"] = json!(answer);
        }
        Some("case_vignette") => {
            resp["verdict"] = json!("agree");
            resp["answers"] = json!(vignette_answers(records, overrides, rid));
        }
        _ => {}
    }
    resp
}

/// The vignette answers the solver claims it derived — the override when
/// set, else a faithful echo of the record's own answers.
fn vignette_answers(
    records: &HashMap<String, Value>,
    overrides: &Overrides,
    rid: &str,
) -> Vec<Value> {
    if let Some(answers) = overrides.solve_vignette_answers.get(rid) {
        return answers.iter().map(|a| json!(a)).collect();
    }
    records[rid]["questions"]
        .as_array()
        .map(|qs| qs.iter().map(|q| q["answer"]["text"].clone()).collect())
        .unwrap_or_default()
}

/// The default fulfiller: solver answers the record's key (or the
/// override), every claim is `supported` when anchored and `no-claim` when
/// not (or the override), distractor and review pass.
pub fn response(records: &HashMap<String, Value>, body: &Value, overrides: &Overrides) -> Value {
    let rid = body["record_id"].as_str().unwrap_or("").to_owned();
    match body["kind"].as_str() {
        Some("solve") => solve_response(records, body, overrides, &rid),
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
pub struct Harness {
    pub dir: PathBuf,
    records: HashMap<String, Value>,
}

impl Harness {
    pub fn new(name: &str) -> Self {
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
    pub fn add_batch_file(&self, rel: &str, text: &str) -> Vec<String> {
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

    pub fn index(&mut self) -> &mut Self {
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

    pub fn config(&self, solver: bool) -> RunConfig {
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

    pub fn opts(&self, config: RunConfig, sources: &Path) -> AdmissionOptions {
        let teachers = teachers_path();
        self.opts_with_teachers(config, sources, &teachers)
    }

    pub fn opts_with_teachers(
        &self,
        config: RunConfig,
        sources: &Path,
        teachers: &Path,
    ) -> AdmissionOptions {
        AdmissionOptions {
            batch_dir: self.dir.join("batch"),
            sources_path: sources.to_path_buf(),
            teachers_path: teachers.to_path_buf(),
            data_dir: self.dir.join("data"),
            log_path: self.dir.join("admission-log.json"),
            config,
        }
    }

    /// Run the pipeline, answering any unanswered request files after each
    /// pass, until a pass produces no new requests — the same loop an
    /// external fulfiller drives.
    pub fn settle(&self, opts: &AdmissionOptions, overrides: &Overrides) -> AdmissionReport {
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
    pub fn request_count(&self, kind: &str) -> usize {
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

pub fn gold_shard(format: &str) -> String {
    fs::read_to_string(
        workspace_root()
            .join("data/gold")
            .join(format!("{format}.jsonl")),
    )
    .unwrap()
}

pub fn sources_path() -> PathBuf {
    workspace_root().join("docs/sources.json")
}

pub fn teachers_path() -> PathBuf {
    workspace_root().join("docs/teachers.md")
}

/// Exact pin block from `docs/teachers.md`, canonicalized by the shared
/// teacher-pin parser used by both generation and admission.
pub fn teacher_pin_entry(pin_id: &str) -> String {
    let text = fs::read_to_string(teachers_path()).unwrap();
    medsynth_schema::teacher_pin::parse_teacher_pin_entries(&text)
        .into_iter()
        .find(|entry| entry.pin_id == pin_id)
        .map(|entry| entry.entry_text)
        .unwrap_or_else(|| panic!("no test pin `{pin_id}` in docs/teachers.md"))
}

pub fn outcome<'a>(
    report: &'a AdmissionReport,
    id: &str,
) -> &'a medsynth_validate::admit::RecordOutcome {
    report
        .outcomes
        .iter()
        .find(|o| o.record_id == id)
        .unwrap_or_else(|| panic!("no outcome for {id}"))
}
