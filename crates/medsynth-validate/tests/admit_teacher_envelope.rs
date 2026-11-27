//! Rights-envelope provenance tests for trusted teacher pins (#9).

mod common;

use common::teacher::{
    DraftCase, PinRejectionCase, TRUSTED_CASE, TRUSTED_PIN, TRUSTED_VARIANT, TeacherCase,
    fabricated_pin_case, run_pin_rejection, settle_prepared_fixture, setup_teacher_case,
    tampered_pin_case, write_teacher_manifest,
};
use common::{outcome, teacher_pin_entry};
use medsynth_validate::admit::Outcome;

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

    for (name, id, route_override, expected_reason) in [
        (
            "route-mismatch",
            "route-mismatch-0001",
            Some("Merge Gateway"),
            "route does not match",
        ),
        (
            "missing-session",
            "missing-session-0001",
            None,
            "missing session_file provenance",
        ),
    ] {
        run_pin_rejection(PinRejectionCase {
            teacher: TeacherCase {
                name,
                draft: DraftCase {
                    id,
                    model: "deepseek-flash",
                },
            },
            teacher_variant: TRUSTED_VARIANT,
            pin_id: TRUSTED_PIN,
            pin_entry: teacher_pin_entry(TRUSTED_PIN),
            route_override,
            include_session: false,
            expected_reason,
        });
    }
}
