use std::collections::{HashMap, HashSet};

use crate::{Check, Finding, Format, RecordFile};

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
    // One pass over the options: label -> content-word overlap. Keep the
    // first score per label — the schema check flags duplicate labels, but
    // a last-wins map would let the duplicate mask a rationale failure.
    let mut scores: HashMap<&str, usize> = HashMap::new();
    for option in options {
        if let Some(label) = option["label"].as_str() {
            scores.entry(label).or_insert_with(|| {
                let option_words = words(option["text"].as_str().unwrap_or(""));
                explained.intersection(&option_words).count()
            });
        }
    }
    let own = scores.get(key).copied().unwrap_or(0);
    let best_other = scores
        .iter()
        .filter(|&(&label, _)| label != key)
        .map(|(_, n)| *n)
        .max()
        .unwrap_or(0);
    if let Some(detail) = key_verdict(explanation, key, own, best_other) {
        push_lint(record, findings, detail);
    }
}

/// `None` when the rationale adequately references the key; `Some(detail)`
/// describes why it does not.
fn key_verdict(explanation: &str, key: &str, own: usize, best_other: usize) -> Option<String> {
    if own == 0 && best_other == 0 {
        return (!references_label(explanation, key))
            .then(|| format!("rationale does not reference key `{key}`"));
    }
    if own <= best_other {
        let why = if own == best_other {
            "does not uniquely explain"
        } else {
            "is not the option the rationale explains"
        };
        return Some(format!(
            "key `{key}` {why} the keyed answer \
             (keyed overlap {own}, best other {best_other})"
        ));
    }
    None
}

/// All-short options (e.g. "ATP"/"ADP") carry no ≥4-letter content words,
/// so overlap cannot discriminate — fall back to the rationale naming the
/// keyed label explicitly. Every key requires the label form ("option B",
/// "answer (C)"): a bare letter is too ambiguous — "a" is an article,
/// "C" a temperature unit, "B"/"D" vitamin names.
fn references_label(explanation: &str, key: &str) -> bool {
    names_option(explanation, key)
}

/// Separators that may precede a label: whitespace, a colon, and the
/// opening markers `(` and quotes.
const SEPARATORS: &[char] = &[' ', '\t', '\u{a0}', ':', '(', '"', '\'', '‘', '’', '“', '”'];
/// Characters that mark a label as enclosed. Closing quotes cannot open:
/// `'option' a …` closes the trigger word, it does not mark `a`.
const MARKERS: &[char] = &['(', '"', '\'', '‘', '“'];
/// A closing quote immediately after the trigger belongs to the word
/// itself ('option' a …), not to a label.
const CLOSING_QUOTES: &[char] = &['"', '\'', '’', '”'];

/// True when the explanation names the keyed label explicitly —
/// "option <key>", "answer <key>", or "choice <key>" — tolerating the
/// separators that legitimately precede a label ("option (A)",
/// "answer: B", "option B"), but not sentence terminators.
fn names_option(explanation: &str, key: &str) -> bool {
    const WORDS: &[&str] = &["option", "answer", "choice"];
    // to_ascii_lowercase preserves byte length, so positions in `lower`
    // still index `explanation` — needed to compare the label in original
    // case below.
    let lower = explanation.to_ascii_lowercase();
    WORDS
        .iter()
        .flat_map(|w| lower.match_indices(w).map(move |(i, _)| (i, w.len())))
        // The trigger word must be whole — not a prefix like "optioned".
        .filter(|(i, len)| {
            lower[..*i]
                .chars()
                .next_back()
                .is_none_or(|c| !c.is_alphanumeric())
                && lower[i + len..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_alphanumeric())
        })
        .any(|(i, len)| label_after_trigger(explanation, i + len, key))
}

/// Examines the text after a trigger word for a reference to `key`.
/// Compare in original case: a bare label must be uppercase, so the
/// article "a" in "answer a question" can never pass. A label inside
/// parens or quotes is marked and may be lowercase ("choice (b)",
/// "option \"a\"").
fn label_after_trigger(explanation: &str, start: usize, key: &str) -> bool {
    let rest = &explanation[start..];
    if rest.starts_with(CLOSING_QUOTES) {
        return false;
    }
    let trimmed = rest.trim_start_matches(|c: char| SEPARATORS.contains(&c));
    // The opener is the last marker before the label — padding between it
    // and the label ("( a )") does not unmark it.
    let opener = rest[..rest.len() - trimmed.len()]
        .trim_end()
        .chars()
        .next_back()
        .filter(|c| MARKERS.contains(c));
    let Some(opener) = opener else {
        return unmarked_label_matches(trimmed, key);
    };
    marked_label_matches(trimmed, key.to_ascii_lowercase().as_str(), opener)
}

/// Bare-label path: `candidate` (original case) must start with the exact
/// uppercase key, followed by a non-alphanumeric boundary.
fn unmarked_label_matches(candidate: &str, key: &str) -> bool {
    candidate.starts_with(key)
        && candidate[key.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric())
}

/// Marked-label path: the lowercased `candidate` must start with `key_cmp`
/// and be closed by the delimiter matching `opener` — "(a)" or `"a"`
/// count, "(a nucleotide" and (a "quote" do not. Whitespace may precede
/// the closer ("( a )"), and a colon may introduce the option text for an
/// uppercase label ("(A: ATP)").
fn marked_label_matches(candidate: &str, key_cmp: &str, opener: char) -> bool {
    let first = candidate.chars().next();
    let candidate = candidate.to_ascii_lowercase();
    if !candidate.starts_with(key_cmp) {
        return false;
    }
    let next = candidate[key_cmp.len()..].trim_start().chars().next();
    match next {
        Some(':') => first.is_some_and(char::is_uppercase),
        Some(c) => closes(opener, c),
        None => false,
    }
}

/// The closer allowed by an opener: `(` pairs only with `)`, straight and
/// curly quotes pair with their own kind (curly singles accept either
/// single-quote closer, curly doubles either double-quote closer).
fn closes(opener: char, closer: char) -> bool {
    match opener {
        '(' => closer == ')',
        '"' => closer == '"',
        '\'' | '‘' => matches!(closer, '\'' | '’'),
        '“' => matches!(closer, '"' | '”'),
        _ => false,
    }
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
        .map(str::to_lowercase)
        .collect()
}
