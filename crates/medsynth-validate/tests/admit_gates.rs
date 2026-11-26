//! Gate-path tests for the admission pipeline (#9): a same-model solver
//! never sees a request, an unpinned solver refuses to start, a stale
//! registry entry holds for re-anchoring, a failed distractor screen
//! rejects at stage 4, and a schema defect rejects at stage 1.
//!
//! Everything runs on scratch copies under the OS temp dir — the gold set
//! and `docs/sources.json` are never mutated.

mod common;

use std::fs;

use medsynth_schema::teacher_pin::teacher_pin_entry_sha256;
use medsynth_validate::admit::{self, AdmissionReport, Outcome};
use serde_json::{Value, json};

use common::{
    Harness, Overrides, SOLVER_MODEL, VERIFIER_MODEL, gold_shard, outcome, sha256_hex,
    sources_path, teacher_pin_entry, workspace_root,
};

const TRUSTED_PIN: &str = "deepseek-v4.1-flash";
const TRUSTED_VARIANT: &str = "DeepSeek-V4.1-Flash-1017";

#[derive(Clone, Copy)]
struct DraftCase {
    id: &'static str,
    model: &'static str,
}

#[derive(Clone, Copy)]
struct TeacherCase {
    name: &'static str,
    draft: DraftCase,
}

struct TeacherFixture {
    harness: Harness,
    case: TeacherCase,
    content_sha: String,
}

struct ManifestSpec<'a> {
    harness: &'a Harness,
    record_id: &'a str,
    content_sha256: &'a str,
}

struct EnvelopeSpec<'a> {
    record_id: &'a str,
    teacher_variant: &'a str,
    model_id: &'a str,
    pin_id: &'a str,
    pin_entry: &'a str,
    content_sha256: &'a str,
}

struct PinRejectionCase {
    teacher: TeacherCase,
    teacher_variant: &'static str,
    pin_id: &'static str,
    pin_entry: String,
    expected_reason: &'static str,
}

const SAME_MODEL_CASE: TeacherCase = TeacherCase {
    name: "same-model",
    draft: DraftCase {
        id: "same-model-draft-0001",
        model: SOLVER_MODEL,
    },
};

const TRUSTED_CASE: TeacherCase = TeacherCase {
    name: "trusted-envelope",
    draft: DraftCase {
        id: "trusted-envelope-0001",
        model: "deepseek-flash",
    },
};

fn write_teacher_record(harness: &Harness, case: &DraftCase, record: &mut Value) -> String {
    record["id"] = json!(case.id);
    record["provenance"]["model"] = json!(case.model);
    record["provenance"]["prompt_version"] = json!("v1");
    let text = serde_json::to_string(&record).unwrap();
    let content_sha = sha256_hex(text.as_bytes());
    harness.add_batch_file(&format!("mcq/{}.json", case.id), &text);
    content_sha
}

fn setup_teacher_case(case: TeacherCase) -> TeacherFixture {
    let harness = Harness::new(case.name);
    let mut record: Value =
        serde_json::from_str(gold_shard("mcq").lines().next().unwrap()).unwrap();
    let content_sha = write_teacher_record(&harness, &case.draft, &mut record);
    TeacherFixture {
        harness,
        case,
        content_sha,
    }
}

fn rights_envelope(spec: &EnvelopeSpec) -> Value {
    json!({
        "record_id": spec.record_id,
        "teacher_variant": spec.teacher_variant,
        "model_id": spec.model_id,
        "pin_id": spec.pin_id,
        "pin_entry": spec.pin_entry,
        "pin_entry_sha256": teacher_pin_entry_sha256(spec.pin_entry),
        "content_sha256": spec.content_sha256,
    })
}

fn trusted_rights_envelope(fixture: &TeacherFixture) -> Value {
    let pin_entry = teacher_pin_entry(TRUSTED_PIN);
    rights_envelope(&EnvelopeSpec {
        record_id: fixture.case.draft.id,
        teacher_variant: TRUSTED_VARIANT,
        model_id: fixture.case.draft.model,
        pin_id: TRUSTED_PIN,
        pin_entry: &pin_entry,
        content_sha256: &fixture.content_sha,
    })
}

fn write_manifest_with_envelope(spec: &ManifestSpec, envelope: Value) {
    fs::write(
        spec.harness.dir.join("batch/manifest.json"),
        serde_json::to_string_pretty(&json!({
            "manifest_version": 1,
            "batch_id": format!("{}-batch", spec.record_id),
            "records": [{
                "id": spec.record_id,
                "lane": "teacher",
                "sha256": spec.content_sha256,
            }],
            "rights_envelopes": [envelope],
        }))
        .unwrap(),
    )
    .unwrap();
}

fn write_teacher_manifest(fixture: &TeacherFixture) {
    write_manifest_with_envelope(
        &ManifestSpec {
            harness: &fixture.harness,
            record_id: fixture.case.draft.id,
            content_sha256: &fixture.content_sha,
        },
        trusted_rights_envelope(fixture),
    );
}

fn settle_prepared_fixture(mut fixture: TeacherFixture) -> (TeacherFixture, AdmissionReport) {
    fixture.harness.index();
    let opts = fixture
        .harness
        .opts(fixture.harness.config(true), &sources_path());
    let report = fixture.harness.settle(&opts, &Overrides::default());
    (fixture, report)
}

fn fabricated_pin_case() -> PinRejectionCase {
    PinRejectionCase {
        teacher: TeacherCase {
            name: "fabricated-pin",
            draft: DraftCase {
                id: "fabricated-pin-0001",
                model: "fabricated-model",
            },
        },
        teacher_variant: "fabricated-model",
        pin_id: "fabricated-pin",
        pin_entry: "fabricated teacher entry".to_owned(),
        expected_reason: "not listed",
    }
}

fn tampered_pin_case() -> PinRejectionCase {
    PinRejectionCase {
        teacher: TeacherCase {
            name: "pin-hash-mismatch",
            draft: DraftCase {
                id: "pin-hash-mismatch-0001",
                model: "deepseek-flash",
            },
        },
        teacher_variant: TRUSTED_VARIANT,
        pin_id: TRUSTED_PIN,
        pin_entry: format!(
            "{}\n- **attacker_added:** `yes`",
            teacher_pin_entry(TRUSTED_PIN)
        ),
        expected_reason: "does not match the trusted teachers registry entry",
    }
}

fn run_pin_rejection(case: PinRejectionCase) {
    let fixture = setup_teacher_case(case.teacher);
    let envelope = rights_envelope(&EnvelopeSpec {
        record_id: fixture.case.draft.id,
        teacher_variant: case.teacher_variant,
        model_id: fixture.case.draft.model,
        pin_id: case.pin_id,
        pin_entry: &case.pin_entry,
        content_sha256: &fixture.content_sha,
    });
    write_manifest_with_envelope(
        &ManifestSpec {
            harness: &fixture.harness,
            record_id: fixture.case.draft.id,
            content_sha256: &fixture.content_sha,
        },
        envelope,
    );
    let (fixture, report) = settle_prepared_fixture(fixture);

    let o = outcome(&report, fixture.case.draft.id);
    assert_eq!((o.outcome, o.stage), (Outcome::Rejected, Some(0)));
    assert!(
        o.reasons.iter().any(|r| r.contains(case.expected_reason)),
        "missing `{}` rejection: {:?}",
        case.expected_reason,
        o.reasons
    );
    assert_eq!(
        fixture.harness.request_count("solve") + fixture.harness.request_count("verify"),
        0
    );
}

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
fn trusted_teacher_envelope_admits() {
    // The positive control: an envelope copied verbatim from the trusted
    // registry clears stage 0 and the record can complete admission.
    let fixture = setup_teacher_case(TRUSTED_CASE);
    write_teacher_manifest(&fixture);
    let (fixture, report) = settle_prepared_fixture(fixture);

    let o = outcome(&report, fixture.case.draft.id);
    assert_eq!(
        (o.outcome, o.stage),
        (Outcome::Admitted, None),
        "trusted envelope must not stop at intake: {:?}",
        o.reasons
    );
    assert_eq!(report.run.teachers_sha256.len(), 64);
}

#[test]
fn fabricated_unlisted_teacher_pin_rejects_at_stage_0() {
    // Self-consistency alone proves nothing: this envelope hashes its own
    // fabricated pin entry correctly, but the pin is not in docs/teachers.md.
    run_pin_rejection(fabricated_pin_case());
}

#[test]
fn teacher_pin_hash_mismatch_rejects_at_stage_0() {
    // The pin is listed, but the envelope embeds a changed entry. Its hash is
    // internally self-consistent, so only comparison with the trusted registry
    // can catch it.
    run_pin_rejection(tampered_pin_case());
}

#[test]
fn missing_or_unparseable_teacher_registry_fails_closed() {
    for (name, contents) in [
        ("missing-teachers", None),
        ("unparseable-teachers", Some("# no pins here\n")),
        ("blank-pin-teachers", Some("### Pin: `blank`\n")),
        (
            "duplicate-pin-teachers",
            Some(
                "### Pin: `one`\n\n- **pin_id:** `same`\n\n### Pin: `two`\n\n- **pin_id:** `same`\n",
            ),
        ),
    ] {
        let mut harness = Harness::new(name);
        harness.add_batch_file("mcq.jsonl", &gold_shard("mcq"));
        harness.index();
        let teachers = harness.dir.join("teachers.md");
        if let Some(contents) = contents {
            fs::write(&teachers, contents).unwrap();
        }
        let opts = harness.opts_with_teachers(harness.config(true), &sources_path(), &teachers);
        let err = admit::run(&opts).expect_err("teacher registry failure must refuse the run");
        assert!(
            err.contains("teacher pin registry"),
            "unexpected error for {name}: {err}"
        );
        assert!(!harness.dir.join("admission-log.json").exists());
        assert_eq!(harness.request_count("solve"), 0);
    }
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
fn teacher_draft(harness: &Harness, case: &DraftCase, record: &mut Value) {
    let content_sha = write_teacher_record(harness, case, record);
    let pin_entry = teacher_pin_entry(TRUSTED_PIN);
    let envelope = rights_envelope(&EnvelopeSpec {
        record_id: case.id,
        teacher_variant: TRUSTED_VARIANT,
        model_id: case.model,
        pin_id: TRUSTED_PIN,
        pin_entry: &pin_entry,
        content_sha256: &content_sha,
    });
    write_manifest_with_envelope(
        &ManifestSpec {
            harness,
            record_id: case.id,
            content_sha256: &content_sha,
        },
        envelope,
    );
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
    assert_eq!(o.outcome, Outcome::Held);
    assert_eq!(o.stage, Some(3));
    assert!(
        o.reasons.iter().any(|r| r.contains("not independent")),
        "missing response-model hold: {:?}",
        o.reasons
    );
}
