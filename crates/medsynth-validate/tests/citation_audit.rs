//! Citation-audit integration coverage (#26): gold adjudication, seeded
//! unsupported citations, receipt-level-only sources, and fail-closed missing
//! adjudicator responses.

use std::fs;
use std::path::{Path, PathBuf};

use medsynth_validate::citation_audit::{AuditOptions, Verdict, run_audit};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .to_path_buf()
}

fn temp_root(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "medsynth-citation-audit-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(path.join("data/gold")).expect("create temp corpus");
    path
}

fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn write_passage_cache(root: &Path) {
    let locator = "sec4.2:epithelial-tissue";
    let text = "Cell to Cell Junctions. A gap junction forms an intercellular passageway between the membranes of adjacent cells to facilitate the movement of small molecules and ions between the cytoplasm of adjacent cells. These junctions allow electrical and metabolic coupling of adjacent cells. Tight junctions block movement between cells.";
    let cache = serde_json::json!({
        "cache_version": 2,
        "source_id": "openstax-anatomy-physiology-2e",
        "locator": locator,
        "retrieval": "openstax-web",
        "book": "anatomy-and-physiology-2e",
        "url": "https://openstax.org/books/anatomy-and-physiology-2e/pages/4-2-epithelial-tissue",
        "text": text,
        "passage_sha256": sha256(text),
    });
    let dir = root.join("cache").join("openstax-anatomy-physiology-2e");
    fs::create_dir_all(&dir).expect("create cache dir");
    fs::write(
        dir.join(format!("{}.json", sha256(locator))),
        serde_json::to_vec_pretty(&cache).expect("serialize cache"),
    )
    .expect("write passage cache");
}

fn options(root: &Path, gold_review: Option<PathBuf>) -> AuditOptions {
    AuditOptions {
        root: root.to_path_buf(),
        records: root.join("data/gold/mcq.jsonl"),
        access: workspace_root().join("docs/citation-access.json"),
        cache_dir: root.join("cache"),
        fulfillment_dir: root.join("fulfillment"),
        gold_review,
        drafting_run_id: Some("drafting-run".to_owned()),
        offline: true,
    }
}

fn write_negative_record(root: &Path) {
    let fixture = workspace_root().join("fixtures/citation-audit/negative-unsupported-mcq.json");
    let value: Value =
        serde_json::from_str(&fs::read_to_string(fixture).expect("read negative fixture"))
            .expect("parse negative fixture");
    let line = serde_json::to_string(&value).expect("serialize negative fixture");
    fs::write(root.join("data/gold/mcq.jsonl"), format!("{line}\n")).expect("write record shard");
}

fn fulfill_requests(root: &Path, choose_verdict: impl Fn(&str) -> &'static str) {
    let fulfillment = root.join("fulfillment");
    for entry in fs::read_dir(&fulfillment).expect("read fulfillment dir") {
        let path = entry.expect("dir entry").path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.ends_with(".audit.request.json") {
            continue;
        }
        let key = name
            .strip_suffix(".audit.request.json")
            .expect("request suffix");
        let request: Value =
            serde_json::from_str(&fs::read_to_string(&path).expect("read request"))
                .expect("parse request");
        let request_sha256 = request["request_sha256"]
            .as_str()
            .expect("request carries its payload hash");
        let claim_path = request["claim_path"].as_str().expect("request claim path");
        let response = serde_json::json!({
            "request_sha256": request_sha256,
            "verdict": choose_verdict(claim_path),
            "run_id": "independent-audit-test-run",
            "detail": "seeded test adjudication",
        });
        fs::write(
            fulfillment.join(format!("{key}.audit.response.json")),
            serde_json::to_vec_pretty(&response).expect("serialize response"),
        )
        .expect("write response");
    }
}

fn fulfill_all(root: &Path, verdict: &'static str) {
    fulfill_requests(root, move |_| verdict);
}

fn fulfill_negative_fixture_mixed(root: &Path) {
    fulfill_requests(root, |claim_path| {
        if claim_path == "/rationale/key_explanation" {
            "unsupported"
        } else {
            "supported"
        }
    });
}

type AnchorMultiset = std::collections::BTreeMap<(String, String, String), usize>;

fn add_json_anchors(record_id: &str, value: &Value, anchors: &mut AnchorMultiset) {
    match value {
        Value::Object(object) => {
            if let Some(Value::Array(items)) = object.get("anchors") {
                for anchor in items {
                    let key = (
                        record_id.to_owned(),
                        anchor["source_id"].as_str().unwrap_or_default().to_owned(),
                        anchor["locator"].as_str().unwrap_or_default().to_owned(),
                    );
                    *anchors.entry(key).or_default() += 1;
                }
            }
            for child in object.values() {
                add_json_anchors(record_id, child, anchors);
            }
        }
        Value::Array(items) => {
            for child in items {
                add_json_anchors(record_id, child, anchors);
            }
        }
        _ => {}
    }
}

fn gold_anchor_multiset() -> AnchorMultiset {
    let mut anchors = AnchorMultiset::new();
    for entry in fs::read_dir(workspace_root().join("data/gold")).expect("read gold dir") {
        let path = entry.expect("gold dir entry").path();
        if path.extension().is_none_or(|ext| ext != "jsonl") {
            continue;
        }
        for line in fs::read_to_string(&path).expect("read gold shard").lines() {
            let record: Value = serde_json::from_str(line).expect("parse gold record");
            add_json_anchors(
                record["id"].as_str().expect("gold record id"),
                &record,
                &mut anchors,
            );
        }
    }
    anchors
}

#[test]
fn gold_review_manifest_marks_hash_identical_gold_anchors_supported() {
    let root = temp_root("gold");
    write_passage_cache(&root);
    let source = workspace_root().join("data/gold/mcq.jsonl");
    let first_line = fs::read_to_string(source)
        .expect("read gold shard")
        .lines()
        .next()
        .expect("gold shard has a record")
        .to_owned();
    fs::write(root.join("data/gold/mcq.jsonl"), format!("{first_line}\n"))
        .expect("write one gold record");

    let review: Value = serde_json::from_str(
        &fs::read_to_string(workspace_root().join("data/gold/review-manifest.json"))
            .expect("read review manifest"),
    )
    .expect("parse review manifest");
    let mut review = review;
    review["records"] = Value::Array(vec![review["records"][0].clone()]);
    let review_path = root.join("review-manifest.json");
    fs::write(
        &review_path,
        serde_json::to_vec_pretty(&review).expect("serialize review"),
    )
    .expect("write review");

    let report = run_audit(&options(&root, Some(review_path.clone()))).expect("run audit");
    assert_eq!(
        (
            report.counts.anchors,
            report.counts.supported,
            report.counts.unsupported,
            report.counts.unverifiable
        ),
        (3, 3, 0, 0)
    );
    assert!(report.unverifiable_by_design_sources.is_empty());
    assert!(
        report
            .anchors
            .iter()
            .all(|a| a.passage_sha256.is_some() && a.adjudicator_run_id.is_some())
    );

    fulfill_all(&root, "unsupported");
    let rebound = run_audit(&options(&root, Some(review_path))).expect("audit reads responses");
    assert_eq!(
        (rebound.counts.supported, rebound.counts.unsupported),
        (0, 3),
        "a hash-bound response for the current passage must override the gold shortcut"
    );
}

#[test]
fn seeded_unsupported_fixture_is_caught_and_carries_passage_hash() {
    let root = temp_root("negative");
    write_passage_cache(&root);
    write_negative_record(&root);

    let first = run_audit(&options(&root, None)).expect("first audit writes requests");
    assert_eq!(first.counts.unverifiable, 3);
    fulfill_negative_fixture_mixed(&root);

    let report = run_audit(&options(&root, None)).expect("second audit reads responses");
    assert_eq!(
        (
            report.counts.anchors,
            report.counts.supported,
            report.counts.unsupported
        ),
        (3, 2, 1)
    );
    assert!(report.anchors.iter().all(|a| {
        let expected = if a.claim_path == "/rationale/key_explanation" {
            Verdict::Unsupported
        } else {
            Verdict::Supported
        };
        a.verdict == expected && a.passage_sha256.is_some()
    }));
    assert_eq!(
        report.release_blocking_record_ids,
        vec!["negative-citation-unsupported-mcq".to_owned()]
    );
}

#[test]
fn missing_adjudicator_response_is_unverifiable_not_supported() {
    let root = temp_root("missing");
    write_passage_cache(&root);
    write_negative_record(&root);
    let report = run_audit(&options(&root, None)).expect("run audit");
    assert_eq!(report.counts.supported, 0);
    assert_eq!(report.counts.unverifiable, 3);
    assert!(report.anchors.iter().all(|a| {
        a.detail
            .contains("missing independent adjudicator response")
    }));
}

#[test]
fn receipt_level_only_sources_are_named_not_skipped() {
    let root = temp_root("receipt");
    write_negative_record(&root);
    let access = serde_json::json!({"version": 1, "sources": []});
    let access_path = root.join("access.json");
    fs::write(
        &access_path,
        serde_json::to_vec_pretty(&access).expect("serialize access"),
    )
    .expect("write empty access policy");
    let mut opts = options(&root, None);
    opts.access = access_path;

    let report = run_audit(&opts).expect("run audit");
    assert_eq!((report.counts.anchors, report.counts.unverifiable), (3, 3));
    assert_eq!(
        report.unverifiable_by_design_sources,
        vec!["openstax-anatomy-physiology-2e".to_owned()]
    );
    assert!(
        report
            .anchors
            .iter()
            .all(|a| a.access == "receipt-level-only")
    );
}

#[test]
fn access_policy_entries_exist_in_the_source_registry() {
    let access: Value = serde_json::from_str(
        &fs::read_to_string(workspace_root().join("docs/citation-access.json"))
            .expect("read access policy"),
    )
    .expect("parse access policy");
    let registry: Value = serde_json::from_str(
        &fs::read_to_string(workspace_root().join("docs/sources.json"))
            .expect("read source registry"),
    )
    .expect("parse source registry");
    let registry_ids = registry["sources"]
        .as_array()
        .expect("registry sources array")
        .iter()
        .filter_map(|entry| entry["source_id"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    for entry in access["sources"].as_array().expect("access sources array") {
        let id = entry["source_id"].as_str().expect("access source id");
        assert!(
            registry_ids.contains(id),
            "access policy source `{id}` is absent from docs/sources.json"
        );
        assert_eq!(
            (entry["retrieval"].as_str(), entry["license"].as_str()),
            (Some("openstax-web"), Some("CC BY-NC-SA 4.0"))
        );
        assert!(
            entry["license_evidence"]
                .as_str()
                .is_some_and(|url| url.starts_with("https://openstax.org/"))
        );
    }
}

#[test]
fn committed_gold_report_is_complete_deterministic_and_text_free() {
    let path = workspace_root().join("reports/citation-audit/gold.json");
    let text = fs::read_to_string(&path).expect("read committed gold report");
    assert!(!text.contains("passage_text"));
    let report: Value = serde_json::from_str(&text).expect("parse committed gold report");
    assert_eq!(
        (
            report["counts"]["records"].as_u64(),
            report["counts"]["anchors"].as_u64(),
            report["counts"]["supported"].as_u64(),
            report["counts"]["unsupported"].as_u64(),
            report["counts"]["unverifiable"].as_u64()
        ),
        (Some(24), Some(89), Some(89), Some(0), Some(0))
    );
    assert!(
        report["release_blocking_record_ids"]
            .as_array()
            .is_some_and(Vec::is_empty)
            && report["unverifiable_by_design_sources"]
                .as_array()
                .is_some_and(Vec::is_empty)
    );
    let anchors = report["anchors"].as_array().expect("anchors array");
    assert!(anchors.iter().all(|anchor| {
        anchor["verdict"] == "supported"
            && anchor["passage_sha256"]
                .as_str()
                .is_some_and(|hash| hash.len() == 64)
            && anchor["adjudicator_run_id"]
                .as_str()
                .is_some_and(|id| !id.is_empty())
            && !anchor["record_path"]
                .as_str()
                .expect("record path")
                .contains('\\')
    }));
    let mut actual = AnchorMultiset::new();
    for anchor in anchors {
        let key = (
            anchor["record_id"].as_str().unwrap_or_default().to_owned(),
            anchor["source_id"].as_str().unwrap_or_default().to_owned(),
            anchor["locator"].as_str().unwrap_or_default().to_owned(),
        );
        *actual.entry(key).or_default() += 1;
    }
    let passages = anchors
        .iter()
        .filter_map(|anchor| anchor["passage_sha256"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        (anchors.len(), passages.len(), actual),
        (89, 17, gold_anchor_multiset())
    );
}

#[test]
fn citation_audit_cli_exits_nonzero_on_unsupported_and_writes_report() {
    let root = temp_root("cli");
    write_passage_cache(&root);
    write_negative_record(&root);
    let report_path = root.join("report.json");
    let args = |report_path: &Path| {
        vec![
            "--root".into(),
            root.display().to_string(),
            "--records".into(),
            root.join("data/gold/mcq.jsonl").display().to_string(),
            "--access".into(),
            workspace_root()
                .join("docs/citation-access.json")
                .display()
                .to_string(),
            "--cache-dir".into(),
            root.join("cache").display().to_string(),
            "--fulfillment-dir".into(),
            root.join("fulfillment").display().to_string(),
            "--drafting-run-id".into(),
            "drafting-run".into(),
            "--offline".into(),
            "--report".into(),
            report_path.display().to_string(),
        ]
    };

    let bin = env!("CARGO_BIN_EXE_medsynth-citation-audit");
    let first = std::process::Command::new(bin)
        .args(args(&report_path))
        .output()
        .expect("first cli run");
    assert!(
        first.status.success(),
        "unverifiable is reported, not blocking"
    );
    fulfill_all(&root, "unsupported");

    let second = std::process::Command::new(bin)
        .args(args(&report_path))
        .output()
        .expect("second cli run");
    assert!(
        !second.status.success(),
        "unsupported must be release-blocking"
    );
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(stderr.contains("unsupported citation"), "stderr: {stderr}");
    let report: Value =
        serde_json::from_str(&fs::read_to_string(&report_path).expect("read cli report"))
            .expect("parse cli report");
    assert_eq!(report["counts"]["unsupported"], 3);
    assert_eq!(
        report["release_blocking_record_ids"],
        serde_json::json!(["negative-citation-unsupported-mcq"])
    );
}
