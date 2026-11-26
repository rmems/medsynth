//! Stage 2: a pinned solver re-derives the answer key from the blind
//! item (the record minus its key/rationale). The solver must not be the
//! model or run that drafted the record, and the check runs before any
//! request is emitted — a same-model solver never sees the item.

use serde_json::Value;

use crate::Format;

use super::batch::DraftRecord;
use super::claims;
use super::eval::{Ctx, Verdict, detail_suffix, identity_mismatch, respond, same_model};
use super::fulfill::RequestKind;

/// Re-derive the key and compare — or hold when the solver cannot answer.
pub(super) fn stage_key(ctx: &Ctx, record: &DraftRecord) -> Result<Verdict, String> {
    let Some(item) = claims::blind_item(record) else {
        return Ok(Verdict::Pass); // tutoring_example: no key to re-derive
    };
    let solver = ctx
        .config
        .solver
        .as_ref()
        .expect("check_pins ran before evaluate");

    // The solver must never be the model/run that drafted the record.
    // Check the rights envelope first, then provenance — before emitting a
    // solve request at all.
    if same_model(ctx, record, &solver.model_id, solver.pin_id.as_deref()) {
        return Ok(Verdict::reject(
            "stage-2 solver is the model/run that drafted this record",
        ));
    }

    let body = claims::solve_body(&ctx.scope(), record, &item);
    let value = match respond(ctx, RequestKind::Solve, &body, "solver")? {
        Ok(value) => value,
        Err(reason) => return Ok(Verdict::hold(reason)),
    };
    if let Some(reason) =
        identity_mismatch(&value, &solver.run_id, Some(&solver.model_id), "solver")
    {
        return Ok(Verdict::hold(reason));
    }
    Ok(compare_key(record, &value))
}

/// Stage 2's verdict on a usable solver response.
fn compare_key(record: &DraftRecord, value: &Value) -> Verdict {
    match record.format {
        Format::Mcq => {
            let key = record.value["key"].as_str().unwrap_or("");
            match value["answer"].as_str() {
                Some(answer) if answer == key => Verdict::Pass,
                Some(answer) => Verdict::reject(format!(
                    "solver derived `{answer}` but the record keys `{key}`"
                )),
                None => Verdict::hold("solver response carries no `answer`"),
            }
        }
        Format::CaseVignette => vignette_verdict(record, value),
        Format::TutoringExample => Verdict::Pass,
    }
}

/// Stage 2's verdict for a vignette: the solver must actually re-derive
/// the answers, and each derivation is compared with the record's answer
/// — a bare `agree` proves nothing (issue #9: disagreement = reject).
fn vignette_verdict(record: &DraftRecord, value: &Value) -> Verdict {
    let Some(questions) = record.value["questions"].as_array() else {
        return Verdict::hold("record carries no `questions` array");
    };
    let derived = value["answers"].as_array();
    if !derived.is_some_and(|a| a.len() == questions.len() && a.iter().all(answer_present)) {
        return Verdict::hold(format!(
            "solver response carries no derived `answers` for the {} questions",
            questions.len()
        ));
    }
    let derived = derived.expect("checked above");
    if let Some(idx) = questions.iter().enumerate().find_map(|(i, q)| {
        let missing = q["answer"]["text"]
            .as_str()
            .is_none_or(|t| t.trim().is_empty());
        missing.then_some(i + 1)
    }) {
        return Verdict::reject(format!(
            "question {idx} carries no answer text — the key cannot be verified"
        ));
    }
    if let Some((idx, want, got)) = questions.iter().enumerate().find_map(|(i, q)| {
        let want = q["answer"]["text"].as_str().unwrap_or("");
        let got = derived[i].as_str().unwrap_or("");
        (!answers_agree(got, want)).then(|| (i + 1, want.to_owned(), got.to_owned()))
    }) {
        return Verdict::reject(format!(
            "solver derived `{got}` but question {idx} answers `{want}`"
        ));
    }
    match value["verdict"].as_str() {
        Some("agree") => Verdict::Pass,
        Some("disagree") => Verdict::reject(format!(
            "solver disagrees with the expected answer{}",
            detail_suffix(value)
        )),
        _ => Verdict::hold("solver response carries no `verdict`"),
    }
}

/// Free-text agreement: the shorter normalized token sequence appears
/// contiguously in the longer one. Token granularity matters — substring
/// matching equates `stratified` with `pseudostratified`, the exact
/// confusion this gate exists to catch — and contiguity rules out
/// coincidental overlap. An independently derived answer rarely matches
/// the record's prose token-for-token beyond the shared conclusion, so a
/// correct paraphrase with no contiguous span is the solver's `verdict`
/// to account for, not the gate's to guess: a mismatch rejects with both
/// texts recorded.
fn answers_agree(derived: &str, expected: &str) -> bool {
    let tokenize = |s: &str| {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    // Exact normalized equality first — purely symbolic answers (`↑`)
    // tokenize to nothing, but identical symbols plainly agree.
    let collapse = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    if collapse(derived) == collapse(expected) {
        return true;
    }
    let (d, e) = (tokenize(derived), tokenize(expected));
    if d.is_empty() || e.is_empty() {
        return false;
    }
    let (short, long) = if d.len() <= e.len() {
        (&d, &e)
    } else {
        (&e, &d)
    };
    long.windows(short.len()).any(|w| w == short.as_slice())
}

/// A derived-answer entry that carries a non-blank string.
fn answer_present(x: &Value) -> bool {
    x.as_str().is_some_and(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::answers_agree;

    #[test]
    fn symbolic_answers_compare_exactly() {
        // Purely symbolic answers tokenize to nothing — identical symbols
        // agree, different ones do not.
        assert!(answers_agree("↑", "↑"));
        assert!(!answers_agree("↑", "↓"));
        assert!(answers_agree("↑ ", " ↑"));
    }

    #[test]
    fn token_containment_not_substring() {
        // The pseudo-prefix trap: `stratified` is a substring of
        // `pseudostratified` but never the same token.
        assert!(!answers_agree(
            "stratified epithelium",
            "pseudostratified epithelium"
        ));
        // A correct restatement with the conclusion embedded passes.
        assert!(answers_agree(
            "The answer is pseudostratified epithelium: the cells form one layer despite appearances",
            "Pseudostratified epithelium: the cells form one layer"
        ));
        // Inserting even one token inside the sequence breaks contiguity.
        assert!(!answers_agree(
            "pseudostratified epithelium, since the cells form one layer",
            "Pseudostratified epithelium: the cells form one layer"
        ));
        // An empty side never agrees.
        assert!(!answers_agree("", "anything"));
        assert!(!answers_agree("anything", ""));
    }
}
