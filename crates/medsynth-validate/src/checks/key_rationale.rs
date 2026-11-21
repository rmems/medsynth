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
/// option — a wrong key leaves the rationale explaining another option.
/// A tie can be resolved by an explicit, unambiguous keyed-label reference:
/// numeric choices and short terms often share every ≥4-letter token.
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
    let explicit_key = scores.contains_key(key) && references_label(explanation, key);
    let unambiguous_label = explicit_key
        && !scores
            .keys()
            .any(|label| *label != key && references_label(explanation, label));
    let labels = LabelSignal {
        explicit_key,
        unambiguous_label,
    };
    if let Some(detail) = key_verdict(key, own, best_other, &labels) {
        push_lint(record, findings, detail);
    }
}

/// Label-reference signals for `key_verdict`: whether the rationale names
/// the keyed label explicitly and whether it names no other label.
struct LabelSignal {
    explicit_key: bool,
    unambiguous_label: bool,
}

/// `None` when the rationale adequately references the key; `Some(detail)`
/// describes why it does not.
fn key_verdict(key: &str, own: usize, best_other: usize, labels: &LabelSignal) -> Option<String> {
    if own == 0 && best_other == 0 {
        // Preserve the existing all-short-option fallback, including a
        // rationale which names a wrong option while refuting it.
        return (!labels.explicit_key).then(|| format!("rationale does not reference key `{key}`"));
    }
    if own == best_other && labels.unambiguous_label {
        return None;
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

/// All-short options (e.g. "ATP"/"ADP"), numeric choices, and shared long
/// words can leave overlap unable to discriminate — fall back to the
/// rationale naming the keyed label explicitly (only that label for a
/// nonzero overlap tie). Every key requires the label form ("option B",
/// "answer (C)"): a bare letter is too ambiguous — "a" is an article,
/// "C" a temperature unit, "B"/"D" vitamin names.
fn references_label(explanation: &str, key: &str) -> bool {
    names_option(explanation, key)
}

/// Separators that may precede a label: whitespace, a colon, and the
/// opening markers `(` and quotes.
const SEPARATORS: &[char] = &[
    ' ', '\t', '\n', '\r', '\u{a0}', ':', '(', '"', '\'', '‘', '’', '“', '”',
];
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
        .any(|(i, len)| {
            LabelTail::after_trigger(&explanation[i + len..])
                .is_some_and(|tail| tail.references(key))
        })
}

/// A label candidate after a trigger word: the text following the
/// separator run plus the enclosing delimiter, if any. A bare label must
/// match the key in original (upper) case, so the article "a" in "answer
/// a question" never passes; a label inside parens or quotes is marked
/// and may be lowercase ("choice (b)", "option \"a\"").
struct LabelTail<'a> {
    text: &'a str,
    opener: Option<char>,
}

impl<'a> LabelTail<'a> {
    /// Parses the tail after a trigger word — `None` when a closing quote
    /// immediately follows the word ('option' a …), which belongs to the
    /// word rather than marking a label.
    fn after_trigger(rest: &'a str) -> Option<Self> {
        if rest.starts_with(CLOSING_QUOTES) {
            return None;
        }
        let text = rest.trim_start_matches(|c: char| SEPARATORS.contains(&c));
        // The opener is the last marker before the label — padding between
        // it and the label ("( a )") does not unmark it.
        let opener = rest[..rest.len() - text.len()]
            .trim_end()
            .chars()
            .next_back()
            .filter(|c| MARKERS.contains(c));
        Some(Self { text, opener })
    }

    /// Whether this tail references `key` as a label.
    fn references(&self, key: &str) -> bool {
        match self.opener {
            None => self.unmarked(key),
            Some(opener) => self.marked(key, opener),
        }
    }

    /// Bare-label path: `text` must start with the exact uppercase key,
    /// followed by a non-alphanumeric boundary.
    fn unmarked(&self, key: &str) -> bool {
        self.text.starts_with(key)
            && self.text[key.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric())
    }

    /// Marked-label path: the label must be closed by the delimiter
    /// matching `opener` — "(a)" or `"a"` count, "(a nucleotide" and
    /// `(a "quote"` do not. Whitespace may precede the closer ("( a )"),
    /// and a colon may introduce the option text for an uppercase label
    /// ("(A: ATP)").
    fn marked(&self, key: &str, opener: char) -> bool {
        let first = self.text.chars().next();
        let text = self.text.to_ascii_lowercase();
        let key = key.to_ascii_lowercase();
        if !text.starts_with(&key) {
            return false;
        }
        match text[key.len()..].trim_start().chars().next() {
            Some(':') => first.is_some_and(char::is_uppercase),
            Some(c) => closes(opener, c),
            None => false,
        }
    }
}

/// The closer allowed by an opener: `(` pairs only with `)`; a quote
/// opener accepts any closer of its family — straight and curly singles
/// interchange, as do straight and curly doubles.
fn closes(opener: char, closer: char) -> bool {
    match opener {
        '(' => closer == ')',
        '"' | '“' => matches!(closer, '"' | '”'),
        '\'' | '‘' => matches!(closer, '\'' | '’'),
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
