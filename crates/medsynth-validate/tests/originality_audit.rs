//! Originality-audit integration coverage (#11): external overlap,
//! dispositions, same-cell dedup, and split-aware contamination groups.

use std::fs;
use std::path::{Path, PathBuf};

use medsynth_validate::originality_audit::{
    AuditOptions, DEFAULT_CONTAMINATION_THRESHOLD, DEFAULT_DEDUP_THRESHOLD,
    DEFAULT_OVERLAP_THRESHOLD, report_json, run_audit,
};
use serde_json::json;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .to_path_buf()
}

fn temp_root(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "medsynth-originality-audit-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("corpus")).expect("create temp corpus");
    path
}

fn mcq(id: &str, stem: &str, locator: &str) -> String {
    serde_json::to_string(&json!({
        "id": id,
        "domain": "cpf",
        "tier": "application",
        "stem": {
            "text": stem,
            "anchors": [{
                "source_id": "ostp-introductory-physics-2019",
                "locator": locator,
            }],
        },
    }))
    .expect("serialize mcq")
}

fn write_records(root: &Path, records: &[String]) {
    fs::write(
        root.join("corpus/mcq.jsonl"),
        records.iter().map(|r| format!("{r}\n")).collect::<String>(),
    )
    .expect("write record shard");
}

fn write_references(root: &Path, texts: &[&str]) -> PathBuf {
    let path = root.join("references.jsonl");
    let mut lines = String::new();
    for (index, text) in texts.iter().enumerate() {
        let line = json!({
            "id": format!("ref-{index}"),
            "kind": "public-exercise",
            "text": text,
            "source_url": "https://example.org/public-exercise",
            "license": "CC BY-SA 4.0",
            "provenance": "seeded integration-test reference",
        });
        lines.push_str(&serde_json::to_string(&line).expect("serialize reference"));
        lines.push('\n');
    }
    fs::write(&path, lines).expect("write references");
    path
}

fn options(root: &Path, references: PathBuf) -> AuditOptions {
    AuditOptions {
        root: root.to_path_buf(),
        records: vec![root.join("corpus/mcq.jsonl")],
        references,
        splits: None,
        dispositions: None,
        overlap_threshold: DEFAULT_OVERLAP_THRESHOLD,
        dedup_threshold: DEFAULT_DEDUP_THRESHOLD,
        contamination_threshold: DEFAULT_CONTAMINATION_THRESHOLD,
    }
}

#[test]
fn external_overlap_requires_review_and_report_omits_reference_text() {
    let root = temp_root("external");
    let reference_text = "When a high speed train is traveling at constant speed, is there a net force on the windows from air pressure? No, since the windows are stationary relative to the train, there is no net force on them from air pressure. Yes, there is a net outwards force on the windows from air pressure. Yes, there is a net inwards force on the windows from air pressure.";
    write_records(
        &root,
        &[mcq("rec-external", reference_text, "sec15.3:hydrodynamics")],
    );
    let references = write_references(&root, &[reference_text]);
    let report = run_audit(&options(&root, references)).expect("run audit");

    assert_eq!(report.counts.external_overlap_flags, 1);
    assert_eq!(report.counts.needs_review, 1);
    assert_eq!(report.counts.release_blocking_flags, 1);
    assert_eq!(report.release_blocking_record_ids, ["rec-external"]);
    let serialized = report_json(&report).expect("serialize report");
    assert!(!serialized.contains("high speed train"));
}

#[test]
fn dispositions_bind_to_deterministic_flag_ids() {
    let root = temp_root("dispositions");
    let reference_text = "Your garden hose has a diameter of D = 2 cm. How fast must water flow out of the hose if you are to fill a 5 l bucket in one minute?";
    write_records(
        &root,
        &[mcq(
            "rec-disposition",
            reference_text,
            "sec15.3:hydrodynamics",
        )],
    );
    let references = write_references(&root, &[reference_text]);
    let first = run_audit(&options(&root, references.clone())).expect("first audit");
    let flag_id = first.flags[0].flag_id.clone();

    let dispositions = root.join("dispositions.json");
    fs::write(
        &dispositions,
        serde_json::to_string_pretty(&json!({
            "dispositions": [{
                "flag_id": flag_id,
                "decision": "not-a-duplicate",
                "reviewer_run_id": "reviewer-test-run",
                "rationale": "seeded false-positive review decision",
            }],
        }))
        .expect("serialize dispositions"),
    )
    .expect("write dispositions");
    let mut opts = options(&root, references);
    opts.dispositions = Some(dispositions);
    let second = run_audit(&opts).expect("second audit");

    assert_eq!(second.counts.dispositions_applied, 1);
    assert_eq!(second.counts.release_blocking_flags, 0);
    assert_eq!(second.flags[0].status, "reviewed-not-a-duplicate");
}

#[test]
fn same_cell_dedup_and_cross_split_contamination_are_flagged() {
    let root = temp_root("dedup-split");
    let stem_a = "A force of 12 N acts on a 3 kg cart. What is the acceleration of the cart according to Newton's second law?";
    let stem_b = "A force of 12 N acts on a 3 kg cart. What is the acceleration of the cart under Newton's second law?";
    write_records(
        &root,
        &[
            mcq("rec-a", stem_a, "sec5.2:force"),
            mcq("rec-b", stem_b, "sec5.2:force"),
        ],
    );
    let references = write_references(
        &root,
        &["A completely unrelated public exercise about water pressure in a straw."],
    );
    let splits = root.join("splits.json");
    fs::write(
        &splits,
        serde_json::to_string_pretty(&json!({
            "assignments": {"rec-a": "train", "rec-b": "test"},
        }))
        .expect("serialize splits"),
    )
    .expect("write splits");
    let mut opts = options(&root, references);
    opts.splits = Some(splits);
    let report = run_audit(&opts).expect("run audit");

    assert_eq!(report.counts.internal_dedup_flags, 1);
    assert_eq!(report.counts.contamination_groups, 1);
    assert_eq!(report.counts.split_violation_flags, 1);
    assert!(report.counts.release_blocking_flags >= 2);
    assert_eq!(report.split_audit.status, "checked");
}

#[test]
fn distinct_same_anchor_records_are_not_deduped() {
    let root = temp_root("distinct");
    write_records(
        &root,
        &[
            mcq(
                "rec-force",
                "A force of 12 N acts on a 3 kg cart. What is its acceleration?",
                "sec5.2:force",
            ),
            mcq(
                "rec-energy",
                "A ball falls from rest through a vertical height. Which energy transformation occurs?",
                "sec5.2:force",
            ),
        ],
    );
    let references = write_references(
        &root,
        &["An unrelated public exercise about epithelial tissue organization."],
    );
    let report = run_audit(&options(&root, references)).expect("run audit");

    assert_eq!(report.counts.internal_dedup_flags, 0);
    assert_eq!(report.counts.release_blocking_flags, 0);
}

#[test]
fn real_gold_is_clean_and_sample_batch_exposes_procedural_duplicates() {
    let root = workspace_root();
    let references = root.join("docs/contamination-reference.jsonl");
    let base = |records: Vec<PathBuf>| AuditOptions {
        root: root.clone(),
        records,
        references: references.clone(),
        splits: None,
        dispositions: None,
        overlap_threshold: DEFAULT_OVERLAP_THRESHOLD,
        dedup_threshold: DEFAULT_DEDUP_THRESHOLD,
        contamination_threshold: DEFAULT_CONTAMINATION_THRESHOLD,
    };

    let gold = run_audit(&base(vec![
        root.join("data/gold/mcq.jsonl"),
        root.join("data/gold/case_vignette.jsonl"),
        root.join("data/gold/tutoring_example.jsonl"),
    ]))
    .expect("audit gold corpus");
    assert_eq!(gold.inputs.records, 24);
    assert_eq!(gold.inputs.reference_items, 4);
    assert_eq!(gold.counts.release_blocking_flags, 0);
    let committed = fs::read_to_string(root.join("reports/originality-audit/gold.json"))
        .expect("read committed gold originality report");
    assert_eq!(
        report_json(&gold).expect("serialize regenerated gold report"),
        committed,
        "committed gold originality report is stale"
    );

    let sample = run_audit(&base(vec![root.join("drafts/sample-batch/mcq.jsonl")]))
        .expect("audit sample batch");
    assert_eq!(sample.inputs.records, 8);
    assert_eq!(sample.counts.internal_dedup_flags, 2);
    assert_eq!(sample.counts.contamination_groups, 2);
    assert_eq!(sample.counts.release_blocking_flags, 2);
    assert_eq!(sample.split_audit.status, "not-run-no-split-assignments");
}

#[test]
fn incomplete_split_assignments_fail_closed() {
    let root = temp_root("incomplete-splits");
    write_records(
        &root,
        &[mcq(
            "rec-unassigned",
            "A distinct question about acceleration.",
            "sec5.2:force",
        )],
    );
    let references = write_references(&root, &["An unrelated public exercise."]);
    let splits = root.join("splits.json");
    fs::write(&splits, r#"{"assignments":{"other-record":"train"}}"#).expect("write splits");
    let mut opts = options(&root, references);
    opts.splits = Some(splits);

    let error = run_audit(&opts).expect_err("incomplete split assignments must fail");
    assert!(
        error
            .to_string()
            .contains("split assignments are incomplete")
    );
}

#[test]
fn non_allowlisted_reference_license_fail_closed() {
    let root = temp_root("bad-license");
    write_records(
        &root,
        &[mcq(
            "rec-license",
            "A distinct question about acceleration.",
            "sec5.2:force",
        )],
    );
    let references = root.join("references.jsonl");
    fs::write(
        &references,
        concat!(
            r#"{"id":"bad-ref","kind":"public-exercise","text":"reference","#,
            r#""source_url":"https://example.org/item","license":"CC BY-NC-SA 4.0","#,
            r#""provenance":"seeded incompatible license"}"#,
            "\n"
        ),
    )
    .expect("write bad references");

    let error = run_audit(&options(&root, references)).expect_err("incompatible license must fail");
    assert!(error.to_string().contains("non-allowlisted license"));
}
