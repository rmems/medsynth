mod common;

use common::workspace_root;
use medsynth_generate::{CoverageMatrix, Format, Lane, RunConfig, TargetCell, TeacherSelection};
use medsynth_schema::{Domain, Subdomain, Tier};

fn matrix() -> CoverageMatrix {
    CoverageMatrix::load(&workspace_root().join("docs/taxonomy.md")).unwrap()
}

fn base_cell() -> TargetCell {
    TargetCell {
        domain: Domain::Bbf,
        format: Format::Mcq,
        tier: Tier::Foundation,
        count: 1,
        lane: Lane::Procedural,
        template: Some("fact.alveolar_epithelium".to_owned()),
        subdomain: Subdomain::AnimalStructure,
        resolved_attempts: 0,
        in_flight_attempts: 0,
    }
}

fn config(cells: Vec<TargetCell>) -> RunConfig {
    RunConfig {
        seed: 1,
        cells,
        teacher: None,
    }
}

fn teacher_cell() -> TargetCell {
    let mut cell = base_cell();
    cell.lane = Lane::Teacher;
    cell.template = None;
    cell
}

/// Validation must fail and report `needle`; other errors may also be
/// present, so callers can assert each expected message.
fn assert_invalid(run: &RunConfig, needle: &str) -> Vec<String> {
    let error = run.validate_against(&matrix()).unwrap_err();
    assert!(
        error.iter().any(|e| e.contains(needle)),
        "missing `{needle}` in {error:?}"
    );
    error
}

#[test]
fn valid_procedural_config_passes() {
    config(vec![base_cell()])
        .validate_against(&matrix())
        .unwrap();
}

#[test]
fn duplicate_cells_fail() {
    assert_invalid(
        &config(vec![base_cell(), base_cell()]),
        "duplicate target cell",
    );
}

#[test]
fn domain_subdomain_mismatch_fails() {
    let mut cell = base_cell();
    cell.subdomain = Subdomain::Respiratory;
    assert_invalid(&config(vec![cell]), "does not belong to domain");
}

#[test]
fn zero_count_and_over_cap_counts_fail() {
    let mut zero = base_cell();
    zero.count = 0;
    assert_invalid(&config(vec![zero]), "count must be positive");

    let mut over = base_cell();
    over.count = 55; // target 18, cap 54
    assert_invalid(&config(vec![over]), "3× budget cap");
}

#[test]
fn replacement_counts_above_the_matrix_target_are_allowed() {
    let mut replacement = base_cell();
    replacement.count = 30;
    config(vec![replacement])
        .validate_against(&matrix())
        .unwrap();
}

#[test]
fn procedural_lane_is_mcq_only() {
    let mut cell = base_cell();
    cell.format = Format::CaseVignette;
    assert_invalid(
        &config(vec![cell]),
        "procedural lane only produces `mcq` drafts",
    );
}

#[test]
fn teacher_cells_reject_templates_and_need_a_selection() {
    let mut cell = teacher_cell();
    cell.template = Some("fact.alveolar_epithelium".to_owned());
    assert_invalid(
        &config(vec![cell]),
        "teacher lane must not set a procedural template",
    );

    assert_invalid(&config(vec![teacher_cell()]), "`teacher` is absent");
}

#[test]
fn teacher_selection_requires_a_pin_and_positive_budgets() {
    let mut run = config(vec![teacher_cell()]);
    run.teacher = Some(TeacherSelection {
        pin_id: " ".to_owned(),
        max_prompt_tokens: 0,
        max_output_tokens: 0,
        ..TeacherSelection::default()
    });
    let error = assert_invalid(&run, "pin_id must not be blank");
    assert!(
        error
            .iter()
            .any(|e| e.contains("token budgets must be positive"))
    );

    let mut run = config(vec![teacher_cell()]);
    run.teacher = Some(TeacherSelection {
        pin_id: "deepseek-v4.1-flash".to_owned(),
        ..TeacherSelection::default()
    });
    run.validate_against(&matrix()).unwrap();
}
