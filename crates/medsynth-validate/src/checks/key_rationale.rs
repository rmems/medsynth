use std::collections::HashSet;

use crate::*;

/// `key-rationale` — rationale present, non-empty, referencing the keyed
/// answer; answer fields non-empty; `AnchoredClaim` anchors non-empty.
pub(super) fn key_rationale_check(record: &RecordFile, findings: &mut Vec<Finding>) {
    match record.format {
        Format::Mcq => key_rationale_mcq(record, findings),
        Format::CaseVignette => key_rationale_vignette(record, findings),
        Format::TutoringExample => key_rationale_tutoring(record, findings),
    }
}

fn push_lint(record: &RecordFile, findings: &mut Vec<Finding>, detail: String) {
    findings.push(Finding {
        path: record.path.clone(),
        line: record.line,
        record_id: record.record_id(),
        check: Check::KeyRationale,
        detail,
    });
}

fn key_rationale_mcq(record: &RecordFile, findings: &mut Vec<Finding>) {
    let v = &record.value;
    let Some(rationale) = v.get("rationale") else {
        push_lint(record, findings, "rationale field missing".into());
        return;
    };
    for field in ["key_explanation", "strongest_distractor"] {
        let claim = &rationale[field];
        if claim["text"].as_str().is_none_or(|t| t.trim().is_empty()) {
            push_lint(record, findings, format!("rationale.{field}.text is empty"));
        }
        if claim["anchors"].as_array().is_none_or(|a| a.is_empty()) {
            push_lint(
                record,
                findings,
                format!("rationale.{field} has no anchors"),
            );
        }
    }
    key_references_option(record, v, rationale, findings);
}

/// The rationale must reference the keyed answer. Cheap deterministic
/// proxy: the keyed option must share strictly more content words
/// (lowercased tokens, len ≥ 4) with `key_explanation` than every other
/// option — a wrong key leaves the rationale explaining another option,
/// and a tie gives no evidence the rationale favors the keyed answer.
fn key_references_option(
    record: &RecordFile,
    v: &serde_json::Value,
    rationale: &serde_json::Value,
    findings: &mut Vec<Finding>,
) {
    let (Some(key), Some(options), Some(explanation)) = (
        v.get("key").and_then(|k| k.as_str()),
        v.get("options").and_then(|o| o.as_array()),
        rationale["key_explanation"]["text"].as_str(),
    ) else {
        return; // missing fields are the schema check's findings
    };
    let explained = words(explanation);
    let overlap = |label: &str| -> usize {
        options
            .iter()
            .find(|o| o["label"].as_str() == Some(label))
            .map(|o| words(o["text"].as_str().unwrap_or("")))
            .map(|w| explained.intersection(&w).count())
            .unwrap_or(0)
    };
    let own = overlap(key);
    let best_other = options
        .iter()
        .filter_map(|o| o["label"].as_str())
        .filter(|l| *l != key)
        .map(overlap)
        .max()
        .unwrap_or(0);
    if own == 0 && best_other == 0 {
        // All-short options (e.g. "ATP"/"ADP") carry no ≥4-letter content
        // words, so overlap cannot discriminate — fall back to requiring
        // the rationale to name the keyed label ("option B" or a bare "B").
        let names_key = explanation
            .split(|c: char| !c.is_alphanumeric())
            .any(|t| t == key)
            || explanation
                .to_lowercase()
                .contains(&format!("option {}", key.to_lowercase()));
        if !names_key {
            push_lint(
                record,
                findings,
                format!("rationale does not reference key `{key}`"),
            );
        }
    } else if own <= best_other {
        let why = if own == best_other {
            "does not uniquely explain"
        } else {
            "is not the option the rationale explains"
        };
        push_lint(
            record,
            findings,
            format!(
                "key `{key}` {why} the keyed answer \
                 (keyed overlap {own}, best other {best_other})"
            ),
        );
    }
}

fn key_rationale_vignette(record: &RecordFile, findings: &mut Vec<Finding>) {
    let v = &record.value;
    if let Some(items) = v.get("questions").and_then(|q| q.as_array()) {
        for (i, q) in items.iter().enumerate() {
            lint_claim(
                record,
                findings,
                &format!("questions[{i}].answer"),
                &q["answer"],
            );
        }
    }
    if let Some(items) = v.get("teaching_points").and_then(|q| q.as_array()) {
        for (i, p) in items.iter().enumerate() {
            lint_claim(record, findings, &format!("teaching_points[{i}]"), p);
        }
    }
}

/// `AnchoredClaim`-shaped lint: non-blank text and a non-empty anchors
/// array, at a JSON path named by `label`.
fn lint_claim(
    record: &RecordFile,
    findings: &mut Vec<Finding>,
    label: &str,
    claim: &serde_json::Value,
) {
    if claim["text"].as_str().is_none_or(|t| t.trim().is_empty()) {
        push_lint(record, findings, format!("{label}.text is empty"));
    }
    if claim["anchors"].as_array().is_none_or(|a| a.is_empty()) {
        push_lint(record, findings, format!("{label} has no anchors"));
    }
}

fn key_rationale_tutoring(record: &RecordFile, findings: &mut Vec<Finding>) {
    let response = &record.value["tutor_response"];
    if response["text"]
        .as_str()
        .is_none_or(|t| t.trim().is_empty())
    {
        push_lint(record, findings, "tutor_response.text is empty".into());
    }
}

/// Content words for the key/rationale overlap check: lowercased word
/// tokens of length ≥ 4.
fn words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 4)
        .map(|w| w.to_lowercase())
        .collect()
}
