mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use common::{TempDir, plan_paths, read_json, workspace_root, write_config};
use medsynth_generate::batch::{ExecuteOptions, LaneFilter, execute};
use medsynth_generate::format::Format;
use medsynth_generate::hash::sha256_str;
use medsynth_generate::manifest::RightsEnvelope;
use medsynth_generate::teacher::{
    AnchorPoint, PreparedTeacher, TeacherClient, TeacherCompletion, TeacherError,
    TeacherPinRegistry, TeacherRequest, build_prompt, build_record, parse_chat_completion,
    parse_completion_json, prepare,
};
use medsynth_generate::validate::{DraftValidator, ExpectedProvenance, ValidationContext};
use medsynth_generate::{Lane, SourceRegistry, TargetCell, TeacherSelection, load_plan};
use medsynth_schema::{Domain, RecordId, Subdomain, Tier};
use serde_json::{Value, json};

fn fixture_path(name: &str) -> PathBuf {
    workspace_root()
        .join("crates/medsynth-generate/tests/fixtures/teacher")
        .join(name)
}

fn fixture_content(name: &str) -> String {
    let value = read_json(&fixture_path(name));
    parse_chat_completion(&value).unwrap().content
}

fn docs_pins() -> TeacherPinRegistry {
    TeacherPinRegistry::load(&workspace_root().join("docs/teachers.md")).unwrap()
}

fn teacher_cell(domain: Domain, format: Format, tier: Tier, subdomain: Subdomain) -> TargetCell {
    TargetCell {
        domain,
        format,
        tier,
        count: 1,
        lane: Lane::Teacher,
        template: None,
        subdomain,
        resolved_attempts: 0,
        in_flight_attempts: 0,
    }
}

fn pin_markdown(pin_id: &str, family: &str, lane: &str, permits_distillation: &str) -> String {
    format!(
        "### Pin: `{pin_id}`\n\n\
- **pin_id:** `{pin_id}`\n\
- **provider:** `Test Provider`\n\
- **family:** `{family}`\n\
- **variant:** `Test-Variant-1`\n\
- **model_id:** `test-model`\n\
- **endpoint:** `https://example.invalid/chat`\n\
- **api_key_env:** `MEDSYNTH_TEST_KEY`\n\
- **lane:** `{lane}`\n\
- **terms_document:** `Test Terms`\n\
- **terms_version:** `v1`\n\
- **terms_date:** `2026-01-01`\n\
- **terms_url:** `https://example.invalid/terms`\n\
- **terms_clause:** `Section 1`\n\
- **permits_distillation:** `{permits_distillation}`\n\
- **permits_synthetic_data:** `yes`\n\
- **snapshot_date:** `2026-11-21`\n"
    )
}

fn load_pin_text(text: &str) -> TeacherPinRegistry {
    let dir = TempDir::new("pins");
    let path = dir.path().join("teachers.md");
    std::fs::write(&path, text).unwrap();
    TeacherPinRegistry::load(&path).unwrap()
}

fn validate_teacher(
    format: Format,
    record: &medsynth_generate::GeneratedRecord,
    pin: &medsynth_generate::TeacherPin,
    allowlist: &BTreeSet<(String, String)>,
) {
    let root = workspace_root();
    let validator = DraftValidator::load(&root.join("schemas")).unwrap();
    let registry = SourceRegistry::load(&root.join("docs/sources.json")).unwrap();
    let context = ValidationContext {
        expected: ExpectedProvenance::Teacher {
            pin,
            prompt_version: "teacher-draft-v1",
        },
        registry: &registry,
        anchor_allowlist: Some(allowlist),
    };
    validator
        .validate_raw(format, &record.to_json_string().unwrap(), &context)
        .unwrap();
}

fn allowlist(pairs: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    pairs
        .iter()
        .map(|(source, locator)| ((*source).to_owned(), (*locator).to_owned()))
        .collect()
}

#[test]
fn pin_registry_parses_both_teachers() {
    let pins = docs_pins();
    let deepseek = pins.ensure_teacher("deepseek-v4.1-flash").unwrap();
    assert_eq!(deepseek.variant, "DeepSeek-V4.1-Flash-1017");
    assert_eq!(deepseek.model_id, "deepseek-flash");
    assert_eq!(deepseek.api_key_env, "MEDSYNTH_TEACHER_DEEPSEEK_API_KEY");
    assert!(deepseek.terms_clause.contains("model distillation"));
    assert!(deepseek.entry_text.starts_with("### Pin:"));

    let nemotron = pins.ensure_teacher("nemotron-3-ultra").unwrap();
    assert_eq!(nemotron.variant, "NVIDIA-Nemotron-3-Ultra-550B-A55B-NVFP4");
    assert!(
        nemotron
            .terms_clause
            .to_ascii_lowercase()
            .contains("derivative")
    );
}

#[test]
fn teacher_lane_refuses_an_unpinned_teacher() {
    let pins = docs_pins();
    let error = pins.ensure_teacher("not-a-pinned-teacher").unwrap_err();
    assert!(matches!(error, TeacherError::Unpinned(_)));
}

#[test]
fn teacher_lane_refuses_an_incomplete_pin() {
    let text = pin_markdown("incomplete-pin", "deepseek", "teacher", "yes")
        .lines()
        .filter(|line| !line.contains("terms_clause"))
        .collect::<Vec<_>>()
        .join("\n");
    let pins = load_pin_text(&text);
    let error = pins.ensure_teacher("incomplete-pin").unwrap_err();
    match error {
        TeacherError::IncompletePin { missing, .. } => {
            assert!(missing.contains(&"terms_clause"));
        }
        other => panic!("expected IncompletePin, got {other}"),
    }
}

#[test]
fn teacher_lane_refuses_research_only_and_ineligible_families() {
    let pins = load_pin_text(&pin_markdown(
        "claude-research",
        "anthropic",
        "research_only",
        "yes",
    ));
    let error = pins.ensure_teacher("claude-research").unwrap_err();
    assert!(matches!(error, TeacherError::ResearchOnly(_)));

    let pins = load_pin_text(&pin_markdown("grok-research", "grok", "teacher", "yes"));
    let error = pins.ensure_teacher("grok-research").unwrap_err();
    assert!(matches!(error, TeacherError::ResearchOnly(_)));

    let pins = load_pin_text(&pin_markdown("unknown-family", "acme", "teacher", "yes"));
    let error = pins.ensure_teacher("unknown-family").unwrap_err();
    assert!(matches!(error, TeacherError::IneligibleFamily { .. }));
}

#[test]
fn teacher_lane_refuses_pins_without_synthetic_data_terms() {
    let pins = load_pin_text(&pin_markdown(
        "no-distillation",
        "deepseek",
        "teacher",
        "no",
    ));
    let error = pins.ensure_teacher("no-distillation").unwrap_err();
    assert!(matches!(error, TeacherError::TermsDoNotPermit { .. }));
}

#[test]
fn teacher_lane_checks_the_pin_before_credentials() {
    let pins = docs_pins();
    let lookups = std::cell::Cell::new(0usize);
    let env = |_: &str| {
        lookups.set(lookups.get() + 1);
        Some("should-not-be-read".to_owned())
    };
    let error = prepare(&pins, "missing-pin", &env).unwrap_err();
    assert!(matches!(error, TeacherError::Unpinned(_)));
    assert_eq!(
        lookups.get(),
        0,
        "credentials must not be read before the pin gate"
    );
}

#[test]
fn teacher_lane_refuses_without_credentials() {
    let pins = docs_pins();
    let error = prepare(&pins, "deepseek-v4.1-flash", &|_| None).unwrap_err();
    match error {
        TeacherError::MissingCredentials { env_var, .. } => {
            assert_eq!(env_var, "MEDSYNTH_TEACHER_DEEPSEEK_API_KEY");
        }
        other => panic!("expected MissingCredentials, got {other}"),
    }
}

#[test]
fn recorded_fixtures_parse_into_valid_unverified_records() {
    let pins = docs_pins();
    let pin = pins.ensure_teacher("deepseek-v4.1-flash").unwrap();

    let cases = [
        (
            Format::Mcq,
            "mcq_response.json",
            teacher_cell(
                Domain::Bbf,
                Format::Mcq,
                Tier::Foundation,
                Subdomain::AnimalStructure,
            ),
            allowlist(&[
                ("openstax-anatomy-physiology-2e", "sec22.2:the-lungs"),
                ("openstax-anatomy-physiology-2e", "sec4.2:epithelial-tissue"),
            ]),
        ),
        (
            Format::CaseVignette,
            "vignette_response.json",
            teacher_cell(
                Domain::PreClinical,
                Format::CaseVignette,
                Tier::Application,
                Subdomain::Endocrine,
            ),
            allowlist(&[
                (
                    "openstax-anatomy-physiology-2e",
                    "sec17.3:the-pituitary-gland-and-hypothalamus",
                ),
                ("openstax-anatomy-physiology-2e", "sec26.2:water-balance"),
            ]),
        ),
        (
            Format::TutoringExample,
            "tutoring_response.json",
            teacher_cell(
                Domain::Bbf,
                Format::TutoringExample,
                Tier::Foundation,
                Subdomain::AnimalStructure,
            ),
            allowlist(&[("openstax-anatomy-physiology-2e", "sec22.2:the-lungs")]),
        ),
    ];

    for (index, (format, fixture, cell, allow)) in cases.into_iter().enumerate() {
        let content = parse_completion_json(format, &fixture_content(fixture)).unwrap();
        let id = RecordId::new(format!("draft-fixture-{index}-000")).unwrap();
        let record = build_record(format, content, &cell, id, pin, "teacher-draft-v1").unwrap();
        assert_eq!(
            record.verification_status(),
            medsynth_schema::VerificationStatus::Unverified
        );
        assert_eq!(
            record.provenance().model.as_deref(),
            Some(pin.variant.as_str())
        );
        assert_eq!(
            record.provenance().prompt_version.as_deref(),
            Some("teacher-draft-v1")
        );
        assert_eq!(record.provenance().generator, "teacher-pipeline-v1");
        validate_teacher(format, &record, pin, &allow);
    }
}

#[test]
fn fenced_fixture_parses_and_malformed_fixture_fails() {
    let fenced = fixture_content("fenced_mcq_response.json");
    assert!(parse_completion_json(Format::Mcq, &fenced).is_ok());

    let malformed = fixture_content("malformed_response.json");
    assert!(parse_completion_json(Format::Mcq, &malformed).is_err());
}

#[test]
fn rights_envelope_requires_every_piece() {
    let pins = docs_pins();
    let mut pin = pins.ensure_teacher("deepseek-v4.1-flash").unwrap().clone();
    let raw = "{\"id\":\"draft-test-000\"}";
    let envelope = RightsEnvelope::build("draft-test-000", raw, &pin).unwrap();
    assert_eq!(envelope.content_sha256, sha256_str(raw));
    assert!(envelope.pin_entry.starts_with("### Pin:"));
    assert_eq!(envelope.pin_entry_sha256, pin.entry_sha256);

    pin.variant.clear();
    assert!(RightsEnvelope::build("draft-test-000", raw, &pin).is_err());

    let mut pin = pins.ensure_teacher("deepseek-v4.1-flash").unwrap().clone();
    pin.entry_text.clear();
    assert!(RightsEnvelope::build("draft-test-000", raw, &pin).is_err());
}

#[test]
fn prompt_forbids_borrowed_items_and_limits_anchors() {
    let cell = teacher_cell(
        Domain::Bbf,
        Format::Mcq,
        Tier::Foundation,
        Subdomain::AnimalStructure,
    );
    let anchors = vec![AnchorPoint {
        source_id: "openstax-anatomy-physiology-2e".to_owned(),
        locator: "sec22.2:the-lungs".to_owned(),
        fact: "Simple squamous epithelium forms the alveolar lining.".to_owned(),
    }];
    let selection = TeacherSelection {
        pin_id: "deepseek-v4.1-flash".to_owned(),
        ..TeacherSelection::default()
    };
    let prompt = build_prompt(&cell, &anchors, &[], &selection).unwrap();
    assert!(prompt.token_count > 0);
    assert!(prompt.system.contains("Do not reproduce"));
    assert!(prompt.system.contains("MCAT"));
    assert!(prompt.system.contains("USMLE"));
    assert!(prompt.system.contains("fictional"));
    assert!(prompt.user.contains("Approved anchor points"));
    assert!(prompt.user.contains("proposed_key"));
    assert!(prompt.user.contains("openstax-anatomy-physiology-2e"));
}

struct FakeTeacher {
    content: String,
}

impl TeacherClient for FakeTeacher {
    fn complete(&self, _: &TeacherRequest) -> Result<TeacherCompletion, TeacherError> {
        Ok(TeacherCompletion {
            content: self.content.clone(),
        })
    }
}

fn teacher_run_config(cells: Value) -> Value {
    json!({
        "seed": 11u64,
        "cells": cells,
        "teacher": {"pin_id": "deepseek-v4.1-flash"}
    })
}

#[test]
fn teacher_batch_writes_an_enveloped_unverified_draft_from_a_recorded_response() {
    let config = teacher_run_config(json!([{
        "domain": "bbf",
        "format": "mcq",
        "tier": "foundation",
        "count": 1,
        "lane": "teacher",
        "subdomain": "animal_structure"
    }]));
    let dir = TempDir::new("teacher-valid");
    let config_path = write_config(&dir, &config);
    let plan = load_plan(&plan_paths(&config_path), None).unwrap();

    let content = fixture_content("mcq_response.json");
    let env =
        |name: &str| (name == "MEDSYNTH_TEACHER_DEEPSEEK_API_KEY").then(|| "test-key".to_owned());
    let factory = move |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Ok(Box::new(FakeTeacher {
            content: content.clone(),
        }))
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        env: &env,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert!(output.lane_error.is_none());
    assert_eq!(output.records.len(), 1);
    assert_eq!(output.manifest.teacher_lane.status, "ran");
    assert_eq!(output.manifest.dropped_by_self_validation, 0);
    assert_eq!(output.manifest.rights_envelopes.len(), 1);
    assert!(!output.manifest.anchor_receipts.is_empty());

    let record = &output.records[0];
    let envelope = &output.manifest.rights_envelopes[0];
    assert_eq!(envelope.record_id, record.record.id().as_str());
    assert_eq!(envelope.teacher_variant, "DeepSeek-V4.1-Flash-1017");
    assert!(envelope.pin_entry.starts_with("### Pin:"));
    assert_eq!(envelope.content_sha256, sha256_str(&record.raw));
    assert_eq!(
        record.record.verification_status(),
        medsynth_schema::VerificationStatus::Unverified
    );

    let out_dir = TempDir::new("teacher-valid-out");
    output.write_output(out_dir.path()).unwrap();
    assert_eq!(
        std::fs::read_to_string(out_dir.path().join("mcq.jsonl"))
            .unwrap()
            .lines()
            .count(),
        1
    );
}

#[test]
fn batch_drops_a_draft_with_an_incomplete_rights_envelope() {
    let config = teacher_run_config(json!([{
        "domain": "bbf",
        "format": "mcq",
        "tier": "foundation",
        "count": 1,
        "lane": "teacher",
        "subdomain": "animal_structure"
    }]));
    let dir = TempDir::new("teacher-envelope");
    let config_path = write_config(&dir, &config);
    let mut plan = load_plan(&plan_paths(&config_path), None).unwrap();

    let mut pin = plan
        .pins
        .as_ref()
        .unwrap()
        .get("deepseek-v4.1-flash")
        .unwrap()
        .clone();
    pin.entry_text.clear();
    plan.pins =
        Some(TeacherPinRegistry::from_pins(plan.paths.teachers.clone(), vec![pin]).unwrap());

    let content = fixture_content("mcq_response.json");
    let env =
        |name: &str| (name == "MEDSYNTH_TEACHER_DEEPSEEK_API_KEY").then(|| "test-key".to_owned());
    let factory = move |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Ok(Box::new(FakeTeacher {
            content: content.clone(),
        }))
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        env: &env,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert!(output.records.is_empty());
    assert_eq!(output.manifest.dropped_by_self_validation, 1);
    assert_eq!(output.manifest.dropped[0].stage, "rights_envelope");
    assert!(
        output.manifest.dropped[0]
            .reasons
            .iter()
            .any(|reason| reason.contains("missing docs/teachers.md pin entry"))
    );
    assert!(output.manifest.rights_envelopes.is_empty());

    let out_dir = TempDir::new("teacher-envelope-out");
    output.write_output(out_dir.path()).unwrap();
    assert!(!out_dir.path().join("mcq.jsonl").exists());
}

#[test]
fn teacher_batch_drops_an_over_budget_completion() {
    let config = teacher_run_config(json!([{
        "domain": "bbf",
        "format": "mcq",
        "tier": "foundation",
        "count": 1,
        "lane": "teacher",
        "subdomain": "animal_structure"
    }]));
    let dir = TempDir::new("teacher-over-budget");
    let config_path = write_config(&dir, &config);
    let plan = load_plan(&plan_paths(&config_path), None).unwrap();

    let env =
        |name: &str| (name == "MEDSYNTH_TEACHER_DEEPSEEK_API_KEY").then(|| "test-key".to_owned());
    let factory = |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Ok(Box::new(FakeTeacher {
            content: "x ".repeat(20_000),
        }))
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        env: &env,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert!(output.lane_error.is_none());
    assert!(output.records.is_empty());
    assert_eq!(output.manifest.dropped_by_generation, 1);
    assert_eq!(output.manifest.dropped[0].stage, "teacher_budget");
    assert_eq!(output.manifest.cells[0].status, "all_dropped");
}

#[test]
fn teacher_batch_drops_a_draft_with_an_unapproved_anchor() {
    let config = teacher_run_config(json!([{
        "domain": "psb",
        "format": "mcq",
        "tier": "foundation",
        "count": 1,
        "lane": "teacher",
        "subdomain": "culture_identity"
    }]));
    let dir = TempDir::new("teacher-allowlist");
    let config_path = write_config(&dir, &config);
    let plan = load_plan(&plan_paths(&config_path), None).unwrap();

    let content = fixture_content("mcq_response.json");
    let env =
        |name: &str| (name == "MEDSYNTH_TEACHER_DEEPSEEK_API_KEY").then(|| "test-key".to_owned());
    let factory = move |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Ok(Box::new(FakeTeacher {
            content: content.clone(),
        }))
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        env: &env,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert!(output.records.is_empty());
    assert_eq!(output.manifest.dropped_by_self_validation, 1);
    assert_eq!(output.manifest.dropped[0].stage, "self_validation");
    assert!(
        output.manifest.dropped[0]
            .reasons
            .iter()
            .any(|reason| reason.contains("approved anchor list"))
    );
}

#[test]
fn missing_pin_file_refuses_the_teacher_lane_and_keeps_procedural_output() {
    let config = teacher_run_config(json!([
        {
            "domain": "cpf",
            "format": "mcq",
            "tier": "application",
            "count": 1,
            "lane": "procedural",
            "template": "calc.newtons_second_law",
            "subdomain": "mechanics"
        },
        {
            "domain": "bbf",
            "format": "case_vignette",
            "tier": "foundation",
            "count": 1,
            "lane": "teacher",
            "subdomain": "animal_structure"
        }
    ]));
    let dir = TempDir::new("teacher-missing-pin");
    let config_path = write_config(&dir, &config);
    let mut paths = plan_paths(&config_path);
    paths.teachers = dir.path().join("does-not-exist.md");
    let plan = load_plan(&paths, None).unwrap();
    assert!(plan.pins.is_none());

    let env =
        |name: &str| (name == "MEDSYNTH_TEACHER_DEEPSEEK_API_KEY").then(|| "test-key".to_owned());
    let factory = |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        panic!("teacher client must not be constructed without a pin file")
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        env: &env,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert_eq!(output.records.len(), 1);
    assert!(output.lane_error.is_some());
    assert_eq!(output.manifest.teacher_lane.status, "refused");
    assert!(
        output
            .manifest
            .teacher_lane
            .error
            .as_deref()
            .unwrap()
            .contains("cannot read")
    );
}

#[test]
fn refused_teacher_lane_still_runs_the_procedural_lane() {
    let config = teacher_run_config(json!([
        {
            "domain": "cpf",
            "format": "mcq",
            "tier": "application",
            "count": 1,
            "lane": "procedural",
            "template": "calc.newtons_second_law",
            "subdomain": "mechanics"
        },
        {
            "domain": "bbf",
            "format": "case_vignette",
            "tier": "foundation",
            "count": 1,
            "lane": "teacher",
            "subdomain": "animal_structure"
        }
    ]));
    let dir = TempDir::new("teacher-refused");
    let config_path = write_config(&dir, &config);
    let plan = load_plan(&plan_paths(&config_path), None).unwrap();

    let env = |_: &str| None;
    let factory = |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        panic!("teacher client must not be constructed without credentials")
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        env: &env,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert_eq!(output.records.len(), 1);
    assert!(output.lane_error.is_some());
    assert_eq!(output.manifest.teacher_lane.status, "refused");
    assert!(
        output
            .manifest
            .teacher_lane
            .error
            .as_deref()
            .unwrap()
            .contains("MEDSYNTH_TEACHER_DEEPSEEK_API_KEY")
    );
    let teacher_cell = output
        .manifest
        .cells
        .iter()
        .find(|cell| cell.lane == "teacher")
        .unwrap();
    assert_eq!(teacher_cell.status, "lane_refused");
    assert_eq!(teacher_cell.dispatched, 0);
    let procedural = output
        .manifest
        .cells
        .iter()
        .find(|cell| cell.lane == "procedural")
        .unwrap();
    assert_eq!(procedural.produced, 1);
}
