//! Teacher-lane fixture builders shared by the focused admission tests.

use std::fs;

use medsynth_schema::teacher_pin::teacher_pin_entry_sha256;
use medsynth_validate::admit::{AdmissionReport, Outcome};
use serde_json::{Value, json};

use super::{
    Harness, Overrides, SOLVER_MODEL, gold_shard, outcome, sha256_hex, sources_path,
    teacher_pin_entry,
};

pub const TRUSTED_PIN: &str = "deepseek-v4.1-flash";
pub const TRUSTED_VARIANT: &str = "DeepSeek-V4.1-Flash-1017";

#[derive(Clone, Copy)]
pub struct DraftCase {
    pub id: &'static str,
    pub model: &'static str,
}

#[derive(Clone, Copy)]
pub struct TeacherCase {
    pub name: &'static str,
    pub draft: DraftCase,
}

pub struct TeacherFixture {
    pub harness: Harness,
    pub case: TeacherCase,
    pub content_sha: String,
}

pub struct ManifestSpec<'a> {
    pub harness: &'a Harness,
    pub record_id: &'a str,
    pub content_sha256: &'a str,
}

pub struct EnvelopeSpec<'a> {
    pub record_id: &'a str,
    pub teacher_variant: &'a str,
    pub model_id: &'a str,
    pub pin_id: &'a str,
    pub pin_entry: &'a str,
    pub content_sha256: &'a str,
}

pub struct PinRejectionCase {
    pub teacher: TeacherCase,
    pub teacher_variant: &'static str,
    pub pin_id: &'static str,
    pub pin_entry: String,
    pub route_override: Option<&'static str>,
    pub include_session: bool,
    pub expected_reason: &'static str,
}

pub const SAME_MODEL_CASE: TeacherCase = TeacherCase {
    name: "same-model",
    draft: DraftCase {
        id: "same-model-draft-0001",
        model: SOLVER_MODEL,
    },
};

pub const TRUSTED_CASE: TeacherCase = TeacherCase {
    name: "trusted-envelope",
    draft: DraftCase {
        id: "trusted-envelope-0001",
        model: "deepseek-flash",
    },
};

pub fn write_teacher_record(harness: &Harness, case: &DraftCase, record: &mut Value) -> String {
    record["id"] = json!(case.id);
    record["provenance"]["model"] = json!(case.model);
    record["provenance"]["prompt_version"] = json!("v1");
    let text = serde_json::to_string(&record).unwrap();
    let content_sha = sha256_hex(text.as_bytes());
    harness.add_batch_file(&format!("mcq/{}.json", case.id), &text);
    content_sha
}

pub fn setup_teacher_case(case: TeacherCase) -> TeacherFixture {
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

pub fn rights_envelope(spec: &EnvelopeSpec) -> Value {
    json!({
        "record_id": spec.record_id,
        "teacher_variant": spec.teacher_variant,
        "model_id": spec.model_id,
        "pin_id": spec.pin_id,
        "pin_entry": spec.pin_entry,
        "pin_entry_sha256": teacher_pin_entry_sha256(spec.pin_entry),
        "content_sha256": spec.content_sha256,
        "route": "Hermes via Merge Gateway",
        "serving_vendor": "openrouter",
        "session_file": "session.json",
        "session_sha256": "session-hash",
    })
}

pub fn trusted_rights_envelope(fixture: &TeacherFixture) -> Value {
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

pub fn write_manifest_with_envelope(spec: &ManifestSpec, envelope: Value) {
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

pub fn write_teacher_manifest(fixture: &TeacherFixture) {
    write_manifest_with_envelope(
        &ManifestSpec {
            harness: &fixture.harness,
            record_id: fixture.case.draft.id,
            content_sha256: &fixture.content_sha,
        },
        trusted_rights_envelope(fixture),
    );
}

pub fn settle_prepared_fixture(mut fixture: TeacherFixture) -> (TeacherFixture, AdmissionReport) {
    fixture.harness.index();
    let opts = fixture
        .harness
        .opts(fixture.harness.config(true), &sources_path());
    let report = fixture.harness.settle(&opts, &Overrides::default());
    (fixture, report)
}

pub fn fabricated_pin_case() -> PinRejectionCase {
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
        route_override: None,
        include_session: true,
        expected_reason: "not listed",
    }
}

pub fn tampered_pin_case() -> PinRejectionCase {
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
        route_override: None,
        include_session: true,
        expected_reason: "does not match the trusted teachers registry entry",
    }
}

pub fn run_pin_rejection(case: PinRejectionCase) {
    let fixture = setup_teacher_case(case.teacher);
    let mut envelope = rights_envelope(&EnvelopeSpec {
        record_id: fixture.case.draft.id,
        teacher_variant: case.teacher_variant,
        model_id: fixture.case.draft.model,
        pin_id: case.pin_id,
        pin_entry: &case.pin_entry,
        content_sha256: &fixture.content_sha,
    });
    if let Some(route) = case.route_override {
        envelope["route"] = json!(route);
    }
    if !case.include_session {
        envelope.as_object_mut().unwrap().remove("session_file");
    }
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

/// A teacher-lane draft whose envelope names `model` as its drafting
/// model — the same-model test pattern, parameterized by the model.
pub fn teacher_draft(harness: &Harness, case: &DraftCase, record: &mut Value) {
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
