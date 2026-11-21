//! Unit tests for each gate check — pass and fail cases — plus the
//! originality screen's copied-record case.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use medsynth_validate::{
    Check, Format, RecordFile, Validators, load_registry, originality, run_checks,
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .to_path_buf()
}

fn registry() -> HashSet<String> {
    load_registry(&workspace_root())
}

fn fixture(format: &str, name: &str) -> serde_json::Value {
    let path = workspace_root().join("fixtures").join(format).join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {path:?}: {e}"))
}

fn record(format: Format, name: &str, value: serde_json::Value) -> RecordFile {
    RecordFile {
        path: PathBuf::from(format!("data/{}/{name}.json", format.slug())),
        line: None,
        format,
        raw: serde_json::to_string(&value).expect("serializes"),
        value,
    }
}

fn findings_for(records: &[RecordFile]) -> Vec<medsynth_validate::Finding> {
    run_checks(records, &Validators::compile(), &registry())
}

fn mcq() -> serde_json::Value {
    fixture("mcq", "valid-enzyme-kinetics.json")
}

#[test]
fn clean_mcq_fixture_passes_every_check() {
    let findings = findings_for(&[record(Format::Mcq, "good", mcq())]);
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn every_valid_fixture_passes_every_check() {
    let records = vec![
        record(
            Format::Mcq,
            "enzyme",
            fixture("mcq", "valid-enzyme-kinetics.json"),
        ),
        record(
            Format::CaseVignette,
            "stemi",
            fixture("case_vignette", "valid-inferior-stemi.json"),
        ),
        record(
            Format::TutoringExample,
            "reinforcement",
            fixture("tutoring_example", "valid-reinforcement.json"),
        ),
        record(
            Format::TutoringExample,
            "socratic",
            fixture("tutoring_example", "valid-socratic-questions.json"),
        ),
    ];
    let findings = findings_for(&records);
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
}

#[test]
fn schema_check_fails_invalid_fixture() {
    let bad = fixture("mcq", "invalid-three-options.json");
    let findings = findings_for(&[record(Format::Mcq, "three-options", bad)]);
    assert!(
        findings.iter().any(|f| f.check == Check::Schema),
        "expected a schema finding, got: {findings:?}"
    );
}

#[test]
fn schema_check_fails_duplicate_record_ids() {
    let mut second = mcq();
    second["domain"] = serde_json::json!("bbf");
    let records = vec![
        record(Format::Mcq, "first", mcq()),
        record(Format::Mcq, "second", second),
    ];
    let findings = findings_for(&records);
    assert!(
        findings
            .iter()
            .any(|f| f.check == Check::Schema && f.detail.contains("duplicate record id")),
        "expected a duplicate-id finding, got: {findings:?}"
    );
}

#[test]
fn key_rationale_fails_when_key_disagrees_with_rationale() {
    let mut bad = mcq();
    // Rationale explains option A ("apparent Km increases, Vmax unchanged");
    // re-keying to B must fail key-rationale.
    bad["key"] = serde_json::json!("B");
    let findings = findings_for(&[record(Format::Mcq, "wrong-key", bad)]);
    assert!(
        findings.iter().any(|f| f.check == Check::KeyRationale),
        "expected a key-rationale finding, got: {findings:?}"
    );
}

#[test]
fn key_rationale_fails_on_blank_rationale_text() {
    let mut bad = mcq();
    // Whitespace-only passes the schema's min-length rule; the lint must
    // still name it empty.
    bad["rationale"]["key_explanation"]["text"] = serde_json::json!("   ");
    let findings = findings_for(&[record(Format::Mcq, "blank-rationale", bad)]);
    assert!(
        findings.iter().any(|f| f.check == Check::KeyRationale),
        "expected a key-rationale finding, got: {findings:?}"
    );
}

#[test]
fn key_rationale_fails_on_unanchored_claim() {
    let mut bad = mcq();
    bad["rationale"]["key_explanation"]["anchors"] = serde_json::json!([]);
    let findings = findings_for(&[record(Format::Mcq, "unanchored-rationale", bad)]);
    assert!(
        findings
            .iter()
            .any(|f| f.check == Check::KeyRationale || f.check == Check::Schema),
        "expected a finding, got: {findings:?}"
    );
}

#[test]
fn anchor_presence_fails_above_unverified_with_no_anchors() {
    let mut bad = mcq();
    // Strip every anchor field and claim the record is key-checked.
    fn strip(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                map.retain(|k, _| k != "anchors");
                for (_, v) in map.iter_mut() {
                    strip(v);
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut bad);
    bad["verification_status"] = serde_json::json!("key-checked");
    let findings = findings_for(&[record(Format::Mcq, "no-anchors", bad)]);
    assert!(
        findings.iter().any(|f| f.check == Check::AnchorPresence),
        "expected an anchor-presence finding, got: {findings:?}"
    );
}

#[test]
fn anchor_presence_fails_on_unregistered_source_ids() {
    // An anchor citing a source id outside the registry counts as no
    // anchor (contract §4).
    let mut bad = mcq();
    fn rewrite(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(s)) = map.get_mut("source_id") {
                    *s = "not-in-registry-1x".into();
                }
                for (_, v) in map.iter_mut() {
                    rewrite(v);
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(rewrite),
            _ => {}
        }
    }
    rewrite(&mut bad);
    bad["verification_status"] = serde_json::json!("anchor-checked");
    let findings = findings_for(&[record(Format::Mcq, "bad-sources", bad)]);
    assert!(
        findings.iter().any(|f| f.check == Check::AnchorPresence),
        "unregistered source ids must not satisfy anchor-presence: {findings:?}"
    );
}

#[test]
fn anchor_presence_flags_unlisted_citation_mixed_with_listed() {
    // One registered anchor must not mask an unlisted citation.
    let mut bad = mcq();
    bad["stem"]["anchors"][0]["source_id"] = serde_json::json!("not-in-registry-1x");
    let findings = findings_for(&[record(Format::Mcq, "mixed-sources", bad)]);
    assert!(
        findings
            .iter()
            .any(|f| f.check == Check::AnchorPresence && f.detail.contains("not-in-registry-1x")),
        "unlisted citation must be flagged even beside listed ones: {findings:?}"
    );
}

#[test]
fn key_rationale_label_fallback_boundaries() {
    // All-short options ("ATP" etc.) carry no content words, forcing the
    // label fallback. Each (key, explanation, expect_pass) case pins down
    // which spellings around an explicit label count as a reference.
    fn findings_for_explanation(key: &str, explanation: &str) -> Vec<medsynth_validate::Finding> {
        let mut r = mcq();
        for (i, opt) in ["ATP", "ADP", "AMP", "GTP"].iter().enumerate() {
            r["options"][i]["text"] = serde_json::json!(opt);
        }
        r["key"] = serde_json::json!(key);
        r["rationale"]["key_explanation"]["text"] = serde_json::json!(explanation);
        findings_for(&[record(Format::Mcq, "label-fallback", r)])
    }
    let cases = [
        (
            "A",
            "Option (A) is correct because ATP is the product.",
            true,
        ),
        (
            "A",
            "Option \"A\" is correct because ATP is the product.",
            true,
        ),
        (
            "A",
            "Option\u{201c}A\u{201d} is correct because ATP is the product.",
            true,
        ),
        (
            "A",
            "Option B is wrong; option A fits because ATP is the product.",
            true,
        ),
        ("B", "Answer B is correct — ADP is the product.", true),
        ("C", "Choice (C) fits — AMP is the product.", true),
        (
            "A",
            "ADP is the best option. A phosphate group is released.",
            false,
        ),
        (
            "A",
            "ADP is the best 'option' a student should pick.",
            false,
        ),
        (
            "A",
            "ADP is the best option; a phosphate group is released.",
            false,
        ),
        // A bare token is not a label: "C" is a temperature unit here and
        // "B" a vitamin name — neither counts as a label reference.
        ("C", "Normal body temperature is 37 °C.", false),
        ("B", "Vitamin B deficiency causes fatigue.", false),
    ];
    for (key, explanation, expect_pass) in cases {
        let findings = findings_for_explanation(key, explanation);
        let got = !findings.iter().any(|f| f.check == Check::KeyRationale);
        assert_eq!(
            got, expect_pass,
            "label fallback outcome wrong for {explanation:?}: {findings:?}"
        );
    }
}

#[test]
fn anchor_presence_passes_unverified_with_no_anchors() {
    // An unverified draft with zero anchors must not trip the check — the
    // gate may only flag records above unverified.
    let mut draft = mcq();
    draft["verification_status"] = serde_json::json!("unverified");
    fn strip(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                map.retain(|k, _| k != "anchors");
                for (_, v) in map.iter_mut() {
                    strip(v);
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut draft);
    let findings = findings_for(&[record(Format::Mcq, "draft", draft)]);
    assert!(
        !findings.iter().any(|f| f.check == Check::AnchorPresence),
        "unverified record must not trip anchor-presence: {findings:?}"
    );
}

#[test]
fn originality_flags_copied_record_and_reports_both_sides() {
    let original = record(Format::Mcq, "original", mcq());
    let mut copied = original.value.clone();
    copied["id"] = serde_json::json!("mcq-copy-0001");
    let copy = record(Format::Mcq, "copy", copied);
    let findings = findings_for(&[original, copy]);
    let flagged: std::collections::BTreeSet<_> = findings
        .iter()
        .filter(|f| f.check == Check::Originality)
        .map(|f| f.path.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        flagged.len(),
        2,
        "both records of a copied pair must be flagged: {findings:?}"
    );
}

#[test]
fn originality_passes_distinct_records() {
    let records = vec![
        record(Format::Mcq, "enzyme", mcq()),
        record(
            Format::CaseVignette,
            "stemi",
            fixture("case_vignette", "valid-inferior-stemi.json"),
        ),
        record(
            Format::TutoringExample,
            "reinforcement",
            fixture("tutoring_example", "valid-reinforcement.json"),
        ),
    ];
    let findings = findings_for(&records);
    assert!(
        !findings.iter().any(|f| f.check == Check::Originality),
        "distinct records must not trip originality: {findings:?}"
    );
}

#[test]
fn originality_screen_parameters_are_documented() {
    // The README/module docs pin these: 5-gram shingles, 0.6 containment.
    assert_eq!(originality::NGRAM, 5);
    assert_eq!(originality::THRESHOLD, 0.6);
}
