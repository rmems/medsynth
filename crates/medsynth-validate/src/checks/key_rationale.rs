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
        if !references_label(explanation, key) {
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

/// All-short options (e.g. "ATP"/"ADP") carry no ≥4-letter content words,
/// so overlap cannot discriminate — fall back to the rationale naming the
/// keyed label. Requires the explicit "option <X>" phrasing for `A` so the
/// article "a" cannot pass as a reference, and accepts a bare letter
/// token for `B`–`E`, which have no article collision.
fn references_label(explanation: &str, key: &str) -> bool {
    if names_option(explanation, key) {
        return true;
    }
    key != "A"
        && explanation
            .split(|c: char| !c.is_alphanumeric())
            .any(|t| t == key)
}

/// True when the explanation names "option <key>" — tolerating the
/// separators that legitimately precede a label ("option (A)",
/// "option: B", "option B"), but not sentence terminators.
fn names_option(explanation: &str, key: &str) -> bool {
    const SEPARATORS: &[char] = &[' ', '\t', '\u{a0}', ':', '(', '"', '\'', '‘', '’', '“', '”'];
    let lower = explanation.to_lowercase();
    let key = key.to_lowercase();
    lower
        .match_indices("option")
        // "option" must be a whole word, not a prefix of a longer word
        // like "optioned".
        .filter(|(i, _)| {
            lower[..*i]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric())
                && lower[i + "option".len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_alphanumeric())
        })
        .any(|(i, _)| {
            // A closing quote immediately after "option" belongs to the
            // word itself ('option' a …), not to a label. Opening curly
            // quotes are unambiguous label openers and stay in SEPARATORS.
            const CLOSING_QUOTES: &[char] = &['"', '\'', '’', '”'];
            let rest = &lower[i + "option".len()..];
            if rest.starts_with(CLOSING_QUOTES) {
                return false;
            }
            let rest = rest.trim_start_matches(|c: char| SEPARATORS.contains(&c));
            rest.starts_with(&key)
                && rest[key.len()..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_alphanumeric())
        })
}

fn key_rationale_vignette(record: &RecordFile, findings: &mut Vec<Finding>) {
    lint_claims(record, findings, "questions", "answer");
    lint_claims(record, findings, "teaching_points", "");
}

/// Lint every `AnchoredClaim` under `field` (each item itself when `sub`
/// is empty, else `item[sub]`).
fn lint_claims(record: &RecordFile, findings: &mut Vec<Finding>, field: &str, sub: &str) {
    let Some(items) = record.value.get(field).and_then(|q| q.as_array()) else {
        return;
    };
    for (i, item) in items.iter().enumerate() {
        let claim = if sub.is_empty() { item } else { &item[sub] };
        let label = if sub.is_empty() {
            format!("{field}[{i}]")
        } else {
            format!("{field}[{i}].{sub}")
        };
        lint_claim(record, findings, &label, claim);
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
