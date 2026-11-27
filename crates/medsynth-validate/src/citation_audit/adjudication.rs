//! Independent-adjudication plumbing for citation-support audit: gold-review
//! adaptation and hash-bound request/response files.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;
use serde_json::Value;

use crate::RecordFile;

use super::{Verdict, sha256_text};

#[derive(Debug, Deserialize)]
pub(crate) struct GoldReviewManifest {
    drafting_run_id: String,
    records: Vec<GoldReviewRecord>,
}

#[derive(Debug, Deserialize)]
struct GoldReviewRecord {
    record_id: String,
    reviewer_run_id: String,
    record_sha256: String,
    anchor_check: GoldAnchorCheck,
}

#[derive(Debug, Deserialize)]
struct GoldAnchorCheck {
    outcome: String,
}

pub(crate) fn load_gold_review(
    path: Option<&PathBuf>,
) -> anyhow::Result<Option<GoldReviewManifest>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let text = fs::read_to_string(path)
        .with_context(|| format!("cannot read gold review {}", path.display()))?;
    Ok(Some(serde_json::from_str(&text).with_context(|| {
        format!("cannot parse gold review {}", path.display())
    })?))
}

pub(crate) fn gold_adjudication(
    review: Option<&GoldReviewManifest>,
    record: &RecordFile,
) -> Option<(Verdict, String, String)> {
    let review = review?;
    let item = review
        .records
        .iter()
        .find(|item| item.record_id == record.record_id())?;
    if item.record_sha256 != sha256_text(&record.raw) {
        return None;
    }
    let independent = item.reviewer_run_id != review.drafting_run_id;
    let verdict = if independent && item.anchor_check.outcome == "pass" {
        Verdict::Supported
    } else if item.anchor_check.outcome == "pass" {
        Verdict::Unverifiable
    } else {
        Verdict::Unsupported
    };
    let detail = if independent {
        "gold review anchor check passed".to_owned()
    } else {
        "gold reviewer run id equals drafting run id".to_owned()
    };
    Some((verdict, item.reviewer_run_id.clone(), detail))
}

#[derive(Debug, Deserialize)]
struct AuditResponse {
    request_sha256: String,
    verdict: String,
    run_id: String,
    #[serde(default)]
    detail: String,
}

pub(crate) fn response_verdict(
    fulfillment_dir: &Path,
    key: &str,
    request_sha256: &str,
    drafting_run_id: Option<&str>,
) -> (Verdict, Option<String>, String) {
    let response_path = fulfillment_dir.join(format!("{key}.audit.response.json"));
    let Ok(text) = fs::read_to_string(&response_path) else {
        return (
            Verdict::Unverifiable,
            None,
            "missing independent adjudicator response".to_owned(),
        );
    };
    let Ok(response) = serde_json::from_str::<AuditResponse>(&text) else {
        return (
            Verdict::Unverifiable,
            None,
            "malformed independent adjudicator response".to_owned(),
        );
    };
    if response.request_sha256 != request_sha256 {
        return (
            Verdict::Unverifiable,
            Some(response.run_id),
            "adjudicator response is not bound to the request hash".to_owned(),
        );
    }
    if response.run_id.trim().is_empty()
        || drafting_run_id.is_none_or(|drafting| drafting == response.run_id)
    {
        return (
            Verdict::Unverifiable,
            Some(response.run_id),
            "adjudicator independence is not established".to_owned(),
        );
    }
    let Some(verdict) = Verdict::parse(&response.verdict) else {
        return (
            Verdict::Unverifiable,
            Some(response.run_id),
            format!("unknown adjudicator verdict `{}`", response.verdict),
        );
    };
    (
        verdict,
        Some(response.run_id),
        if response.detail.is_empty() {
            "independent adjudicator response".to_owned()
        } else {
            response.detail
        },
    )
}

pub(crate) fn write_request(
    fulfillment_dir: &Path,
    key: &str,
    request: &Value,
) -> anyhow::Result<()> {
    fs::create_dir_all(fulfillment_dir)?;
    let path = fulfillment_dir.join(format!("{key}.audit.request.json"));
    if !path.exists() {
        fs::write(path, serde_json::to_vec_pretty(request)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "medsynth-adjudication-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    fn write_response(dir: &Path, body: &str) {
        fs::write(dir.join("key.audit.response.json"), body).expect("write response");
    }

    #[test]
    fn response_edge_cases_are_unverifiable() {
        let cases = [
            ("not json", "malformed"),
            (
                r#"{"request_sha256":"wrong","verdict":"supported","run_id":"audit"}"#,
                "not bound",
            ),
            (
                r#"{"request_sha256":"req","verdict":"supported","run_id":"draft"}"#,
                "independence",
            ),
            (
                r#"{"request_sha256":"req","verdict":"maybe","run_id":"audit"}"#,
                "unknown adjudicator verdict",
            ),
        ];
        for (idx, (body, detail)) in cases.into_iter().enumerate() {
            let dir = temp_dir(&format!("case{idx}"));
            write_response(&dir, body);
            let (verdict, _, message) = response_verdict(&dir, "key", "req", Some("draft"));
            assert_eq!(verdict, Verdict::Unverifiable);
            assert!(message.contains(detail), "{message} missing {detail}");
        }
    }

    #[test]
    fn missing_drafting_run_id_does_not_establish_independence() {
        let dir = temp_dir("unknown-drafting");
        write_response(
            &dir,
            r#"{"request_sha256":"req","verdict":"supported","run_id":"audit"}"#,
        );
        let (verdict, _, message) = response_verdict(&dir, "key", "req", None);
        assert_eq!(verdict, Verdict::Unverifiable);
        assert!(message.contains("independence"));
    }

    #[test]
    fn gold_review_requires_exact_record_hash_and_independence() {
        let raw = r#"{"id":"rec"}"#;
        let record = RecordFile {
            path: PathBuf::from("data/gold/mcq.jsonl"),
            line: Some(1),
            format: crate::Format::Mcq,
            value: serde_json::json!({"id":"rec"}),
            raw: raw.to_owned(),
        };
        let mut review: GoldReviewManifest = serde_json::from_value(serde_json::json!({
            "drafting_run_id": "draft",
            "records": [{
                "record_id": "rec",
                "reviewer_run_id": "audit",
                "record_sha256": sha256_text(raw),
                "anchor_check": {"outcome":"pass"}
            }]
        }))
        .expect("review fixture");
        assert_eq!(
            gold_adjudication(Some(&review), &record).map(|v| v.0),
            Some(Verdict::Supported)
        );
        review.drafting_run_id = "audit".to_owned();
        assert_eq!(
            gold_adjudication(Some(&review), &record).map(|v| v.0),
            Some(Verdict::Unverifiable)
        );
        review.records[0].record_sha256 = "different".to_owned();
        assert!(gold_adjudication(Some(&review), &record).is_none());
    }
}
