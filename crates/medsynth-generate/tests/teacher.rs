mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use common::{TempDir, plan_paths, read_json, workspace_root, write_config};
use medsynth_generate::batch::{ExecuteOptions, LaneFilter, execute};
use medsynth_generate::format::Format;
use medsynth_generate::hash::sha256_str;
use medsynth_generate::manifest::RightsEnvelope;
use medsynth_generate::teacher::{
    AnchorPoint, PreparedTeacher, RecordStamp, TeacherClient, TeacherCompletion, TeacherError,
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
- **endpoint:** `https://api.deepseek.com/chat/completions`\n\
- **api_key_env:** `TEST_TEACHER_API_KEY`\n\
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
    assert_eq!(
        (
            deepseek.variant.as_str(),
            deepseek.model_id.as_str(),
            deepseek.api_key_env.as_str(),
        ),
        (
            "DeepSeek-V4.1-Flash-1017",
            "deepseek-flash",
            "DEEPSEEK_API_KEY",
        )
    );
    assert!(
        deepseek.terms_clause.contains("model distillation")
            && deepseek.entry_text.starts_with("### Pin:")
    );

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
fn teacher_lane_refuses_untrusted_endpoints_and_foreign_env_names() {
    // The repo never calls the API, but the pin document is what the harness
    // dispatches on — an untrusted host or foreign credential name is refused.
    let text = pin_markdown("bad-endpoint", "deepseek", "teacher", "yes").replace(
        "https://api.deepseek.com/chat/completions",
        "https://attacker.example/collect",
    );
    let pins = load_pin_text(&text);
    let error = pins.ensure_teacher("bad-endpoint").unwrap_err();
    assert!(matches!(error, TeacherError::UntrustedEndpoint { .. }));

    let text = pin_markdown("bad-env", "deepseek", "teacher", "yes")
        .replace("TEST_TEACHER_API_KEY", "AWS_SECRET_ACCESS_KEY");
    let pins = load_pin_text(&text);
    let error = pins.ensure_teacher("bad-env").unwrap_err();
    assert!(matches!(error, TeacherError::InvalidCredentialEnv { .. }));
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
fn teacher_lane_checks_the_pin_and_needs_no_credentials() {
    let pins = docs_pins();
    let error = prepare(&pins, "missing-pin").unwrap_err();
    assert!(matches!(error, TeacherError::Unpinned(_)));

    // A pinned teacher prepares with no credential lookup at all — the
    // external fulfillment harness owns the key.
    let prepared = prepare(&pins, "deepseek-v4.1-flash").unwrap();
    assert_eq!(prepared.pin().pin_id, "deepseek-v4.1-flash");
}

/// Inputs for one recorded-completion test.
struct FixtureCase {
    format: Format,
    fixture: &'static str,
    cell: TargetCell,
    allow: BTreeSet<(String, String)>,
    index: usize,
}

/// A recorded teacher completion parses, builds, and validates as an
/// unverified draft with full provenance.
fn assert_fixture_record(case: &FixtureCase) {
    let pins = docs_pins();
    let pin = pins.ensure_teacher("deepseek-v4.1-flash").unwrap();
    let content = parse_completion_json(case.format, &fixture_content(case.fixture)).unwrap();
    let id = RecordId::new(format!("draft-fixture-{}-000", case.index)).unwrap();
    let stamp = RecordStamp {
        id,
        pin,
        prompt_version: "teacher-draft-v1",
    };
    let record = build_record(case.format, content, &case.cell, &stamp).unwrap();
    assert_unverified_provenance(&record, pin);
    validate_teacher(case.format, &record, pin, &case.allow);
}

/// The stamp the pipeline — never the teacher — applies to a draft.
fn assert_unverified_provenance(
    record: &medsynth_generate::GeneratedRecord,
    pin: &medsynth_generate::TeacherPin,
) {
    let provenance = record.provenance();
    assert_eq!(
        (
            record.verification_status(),
            provenance.model.as_deref(),
            provenance.prompt_version.as_deref(),
            provenance.generator.as_str(),
        ),
        (
            medsynth_schema::VerificationStatus::Unverified,
            Some(pin.variant.as_str()),
            Some("teacher-draft-v1"),
            "teacher-pipeline-v1",
        )
    );
}

#[test]
fn mcq_fixture_parses_into_a_valid_unverified_record() {
    assert_fixture_record(&FixtureCase {
        format: Format::Mcq,
        fixture: "mcq_response.json",
        cell: teacher_cell(
            Domain::Bbf,
            Format::Mcq,
            Tier::Foundation,
            Subdomain::AnimalStructure,
        ),
        allow: allowlist(&[
            ("openstax-anatomy-physiology-2e", "sec22.2:the-lungs"),
            ("openstax-anatomy-physiology-2e", "sec4.2:epithelial-tissue"),
        ]),
        index: 0,
    });
}

#[test]
fn vignette_fixture_parses_into_a_valid_unverified_record() {
    assert_fixture_record(&FixtureCase {
        format: Format::CaseVignette,
        fixture: "vignette_response.json",
        cell: teacher_cell(
            Domain::PreClinical,
            Format::CaseVignette,
            Tier::Application,
            Subdomain::Endocrine,
        ),
        allow: allowlist(&[
            (
                "openstax-anatomy-physiology-2e",
                "sec17.3:the-pituitary-gland-and-hypothalamus",
            ),
            ("openstax-anatomy-physiology-2e", "sec26.2:water-balance"),
        ]),
        index: 1,
    });
}

#[test]
fn tutoring_fixture_parses_into_a_valid_unverified_record() {
    assert_fixture_record(&FixtureCase {
        format: Format::TutoringExample,
        fixture: "tutoring_response.json",
        cell: teacher_cell(
            Domain::Bbf,
            Format::TutoringExample,
            Tier::Foundation,
            Subdomain::AnimalStructure,
        ),
        allow: allowlist(&[("openstax-anatomy-physiology-2e", "sec22.2:the-lungs")]),
        index: 2,
    });
}

#[test]
fn fenced_fixture_parses_and_malformed_fixture_fails() {
    let fenced = fixture_content("fenced_mcq_response.json");
    assert!(parse_completion_json(Format::Mcq, &fenced).is_ok());

    let malformed = fixture_content("malformed_response.json");
    assert!(parse_completion_json(Format::Mcq, &malformed).is_err());
}

#[test]
fn indented_closing_fence_parses_and_double_fenced_output_fails() {
    let body = fixture_content("mcq_response.json");
    // CommonMark allows up to 3 leading spaces on the closing fence.
    let indented = format!("```json\n{body}\n  ```");
    assert!(parse_completion_json(Format::Mcq, &indented).is_ok());

    let double = format!("```json\n{body}\n```\n```json\n{body}\n```");
    let error = parse_completion_json(Format::Mcq, &double).unwrap_err();
    assert!(
        error.to_string().contains("after the closing fence"),
        "{error}"
    );
}

#[test]
fn closing_fence_must_be_at_least_as_long_as_the_opening_fence() {
    let body = fixture_content("mcq_response.json");
    let long = format!("````json\n{body}\n````");
    assert!(parse_completion_json(Format::Mcq, &long).is_ok());
    // Three backticks cannot close a four-backtick block (CommonMark).
    let short_close = format!("````json\n{body}\n```");
    assert!(parse_completion_json(Format::Mcq, &short_close).is_err());
}

#[test]
fn rights_envelope_requires_every_piece() {
    let pins = docs_pins();
    let mut pin = pins.ensure_teacher("deepseek-v4.1-flash").unwrap().clone();
    let raw = "{\"id\":\"draft-test-000\"}";
    let envelope = RightsEnvelope::build("draft-test-000", raw, &pin, None).unwrap();
    assert_eq!(
        (&envelope.content_sha256, &envelope.pin_entry_sha256),
        (&sha256_str(raw), &pin.entry_sha256)
    );
    assert!(envelope.pin_entry.starts_with("### Pin:"));

    pin.variant.clear();
    assert!(RightsEnvelope::build("draft-test-000", raw, &pin, None).is_err());

    let mut pin = pins.ensure_teacher("deepseek-v4.1-flash").unwrap().clone();
    pin.entry_text.clear();
    assert!(RightsEnvelope::build("draft-test-000", raw, &pin, None).is_err());
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
    assert!(
        prompt.system.contains("Do not reproduce")
            && prompt.system.contains("MCAT")
            && prompt.system.contains("USMLE")
            && prompt.system.contains("fictional"),
        "system prompt must forbid borrowed items"
    );
    assert!(
        prompt.user.contains("Approved anchor points")
            && prompt.user.contains("proposed_key")
            && prompt.user.contains("openstax-anatomy-physiology-2e"),
        "user prompt must list the approved anchors"
    );
}

struct FakeTeacher {
    content: String,
}

impl TeacherClient for FakeTeacher {
    fn complete(&self, _: &TeacherRequest) -> Result<TeacherCompletion, TeacherError> {
        Ok(TeacherCompletion {
            content: self.content.clone(),
            served_model: Some("deepseek-flash".to_owned()),
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

/// One procedural MCQ cell plus one teacher vignette cell — the refusal tests.
fn mixed_lane_cells() -> Value {
    json!([
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
    ])
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
    let factory = move |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Ok(Box::new(FakeTeacher {
            content: content.clone(),
        }))
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert!(output.lane_error.is_none());
    assert_eq!(
        (
            output.records.len(),
            output.manifest.rights_envelopes.len(),
            output.manifest.dropped_by_self_validation,
            output.manifest.teacher_lane.status.as_str(),
        ),
        (1, 1, 0, "ran")
    );
    assert!(!output.manifest.anchor_receipts.is_empty());

    let record = &output.records[0];
    let envelope = &output.manifest.rights_envelopes[0];
    assert_eq!(
        (
            envelope.record_id.as_str(),
            envelope.teacher_variant.as_str(),
            envelope.content_sha256.as_str(),
        ),
        (
            record.record.id().as_str(),
            "DeepSeek-V4.1-Flash-1017",
            sha256_str(&record.raw).as_str(),
        )
    );
    assert!(envelope.pin_entry.starts_with("### Pin:"));
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
    let factory = move |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Ok(Box::new(FakeTeacher {
            content: content.clone(),
        }))
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert!(output.records.is_empty() && output.manifest.rights_envelopes.is_empty());
    assert_eq!(
        (
            output.manifest.dropped_by_self_validation,
            output.manifest.dropped[0].stage.as_str(),
        ),
        (1, "rights_envelope")
    );
    assert!(
        output.manifest.dropped[0]
            .reasons
            .iter()
            .any(|reason| reason.contains("missing docs/teachers.md pin entry"))
    );

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

    let factory = |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Ok(Box::new(FakeTeacher {
            content: "x ".repeat(20_000),
        }))
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert!(output.lane_error.is_none() && output.records.is_empty());
    assert_eq!(
        (
            output.manifest.dropped_by_generation,
            output.manifest.dropped[0].stage.as_str(),
            output.manifest.cells[0].status.as_str(),
        ),
        (1, "teacher_budget", "all_dropped")
    );
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
    let factory = move |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Ok(Box::new(FakeTeacher {
            content: content.clone(),
        }))
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert!(output.records.is_empty());
    assert_eq!(
        (
            output.manifest.dropped_by_self_validation,
            output.manifest.dropped[0].stage.as_str(),
        ),
        (1, "self_validation")
    );
    assert!(
        output.manifest.dropped[0]
            .reasons
            .iter()
            .any(|reason| reason.contains("approved anchor list"))
    );
}

#[test]
fn missing_pin_file_refuses_the_teacher_lane_and_keeps_procedural_output() {
    let config = teacher_run_config(mixed_lane_cells());
    let dir = TempDir::new("teacher-missing-pin");
    let config_path = write_config(&dir, &config);
    let mut paths = plan_paths(&config_path);
    paths.teachers = dir.path().join("does-not-exist.md");
    let plan = load_plan(&paths, None).unwrap();
    assert!(plan.pins.is_none());

    let factory = |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        panic!("teacher client must not be constructed without a pin file")
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert_eq!(
        (
            output.records.len(),
            output.manifest.teacher_lane.status.as_str(),
        ),
        (1, "refused")
    );
    assert!(output.lane_error.is_some());
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
    let config = teacher_run_config(mixed_lane_cells());
    let dir = TempDir::new("teacher-refused");
    let config_path = write_config(&dir, &config);
    let plan = load_plan(&plan_paths(&config_path), None).unwrap();

    let factory = |_: &PreparedTeacher| -> Result<Box<dyn TeacherClient>, TeacherError> {
        Err(TeacherError::MissingFulfillmentDir)
    };
    let options = ExecuteOptions {
        lane_filter: LaneFilter::All,
        teacher_client_factory: &factory,
    };
    let output = execute(&plan, &options).unwrap();
    assert_eq!(
        (
            output.records.len(),
            output.manifest.teacher_lane.status.as_str(),
        ),
        (1, "refused")
    );
    assert!(output.lane_error.is_some());
    assert!(
        output
            .manifest
            .teacher_lane
            .error
            .as_deref()
            .unwrap()
            .contains("fulfillment directory")
    );
    let teacher_cell = output
        .manifest
        .cells
        .iter()
        .find(|cell| cell.lane == "teacher")
        .unwrap();
    let procedural = output
        .manifest
        .cells
        .iter()
        .find(|cell| cell.lane == "procedural")
        .unwrap();
    assert_eq!(
        (
            teacher_cell.status.as_str(),
            teacher_cell.dispatched,
            procedural.produced,
        ),
        ("lane_refused", 0, 1)
    );
}
