//! The record content the external checks see: blind solve items (stage 2),
//! claim blocks for the anchor/support screen (stage 3), and the distractor
//! screen payload (stage 4).
//!
//! Claim blocks are every text block that can carry anchors per the record
//! schemas — `AnchoredText`, `AnchoredClaim`, and `tutor_response`. A block
//! with empty anchors is submitted too: whether its text asserts a fact is
//! the verifier's call (contract §4 exempts pure pedagogy and invented
//! patient observations, and only a semantic check can tell those from an
//! unanchored claim).

use serde_json::{Value, json};

use crate::Format;

use super::batch::DraftRecord;

/// One anchored-or-anchorable text block submitted to the claim verifier.
#[derive(Debug, Clone)]
pub struct ClaimBlock {
    /// JSON path into the record — `/stem`, `/rationale/key_explanation`,
    /// `/questions/0/answer`, `/tutor_response`.
    pub path: String,
    pub text: String,
    /// `(source_id, locator)` pairs the block cites; empty for
    /// `AnchoredText` blocks whose anchors are empty.
    pub anchors: Vec<(String, String)>,
}

/// The distractor screen payload for an `mcq` record.
#[derive(Debug, Clone)]
pub struct DistractorScreen {
    pub options: Vec<(String, String)>,
    pub key: String,
    pub refutation: String,
}

fn anchors_of(block: &Value) -> Vec<(String, String)> {
    block["anchors"]
        .as_array()
        .map(|anchors| {
            anchors
                .iter()
                .filter_map(|a| {
                    Some((
                        a["source_id"].as_str()?.to_owned(),
                        a["locator"].as_str()?.to_owned(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn claim(path: impl Into<String>, block: &Value) -> Option<ClaimBlock> {
    let text = block["text"].as_str()?.to_owned();
    Some(ClaimBlock {
        path: path.into(),
        text,
        anchors: anchors_of(block),
    })
}

/// Every claim-bearing block in the record, in record order.
pub fn claim_blocks(record: &DraftRecord) -> Vec<ClaimBlock> {
    let v = &record.value;
    match record.format {
        Format::Mcq => mcq_claims(v),
        Format::CaseVignette => case_vignette_claims(v),
        Format::TutoringExample => tutoring_example_claims(v),
    }
}

fn mcq_claims(v: &Value) -> Vec<ClaimBlock> {
    let mut out: Vec<ClaimBlock> = [claim("/stem", &v["stem"])].into_iter().flatten().collect();
    // Every option goes to the verifier: anchored ones need their facts
    // supported; unanchored ones the verifier rules `no-claim` or
    // `unanchored` — a fact the refutation doesn't address still must
    // anchor (§4), and only a semantic check can tell which is which.
    out.extend(
        items(v, "options")
            .enumerate()
            .filter_map(|(i, o)| claim(format!("/options/{i}"), o)),
    );
    out.extend(
        [
            claim(
                "/rationale/key_explanation",
                &v["rationale"]["key_explanation"],
            ),
            claim(
                "/rationale/strongest_distractor",
                &v["rationale"]["strongest_distractor"],
            ),
        ]
        .into_iter()
        .flatten(),
    );
    out
}

/// The `value` array's items, or an empty slice when absent or not an array.
fn items<'a>(v: &'a Value, field: &str) -> impl Iterator<Item = &'a Value> {
    v[field].as_array().into_iter().flat_map(|a| a.iter())
}

fn case_vignette_claims(v: &Value) -> Vec<ClaimBlock> {
    let mut out: Vec<ClaimBlock> = items(v, "questions")
        .enumerate()
        .filter_map(|(i, q)| claim(format!("/questions/{i}/answer"), &q["answer"]))
        .collect();
    for field in ["reasoning_chain", "teaching_points"] {
        out.extend(
            items(v, field)
                .enumerate()
                .filter_map(|(i, item)| claim(format!("/{field}/{i}"), item)),
        );
    }
    out
}

fn tutoring_example_claims(v: &Value) -> Vec<ClaimBlock> {
    // The learner's own wording is not a medsynth claim (schema docs);
    // submit it only when it carries anchors of its own.
    let learner = v["learner_prompt"]["anchors"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
        .then(|| claim("/learner_prompt", &v["learner_prompt"]))
        .flatten();
    [learner, claim("/tutor_response", &v["tutor_response"])]
        .into_iter()
        .flatten()
        .collect()
}

/// The distractor screen payload — `mcq` records only.
pub fn distractor_screen(record: &DraftRecord) -> Option<DistractorScreen> {
    if record.format != Format::Mcq {
        return None;
    }
    let v = &record.value;
    let options = v["options"]
        .as_array()?
        .iter()
        .filter_map(|o| {
            Some((
                o["label"].as_str()?.to_owned(),
                o["text"].as_str()?.to_owned(),
            ))
        })
        .collect();
    Some(DistractorScreen {
        options,
        key: v["key"].as_str()?.to_owned(),
        refutation: v["rationale"]["strongest_distractor"]["text"]
            .as_str()
            .unwrap_or("")
            .to_owned(),
    })
}

/// The blind item a solver sees: everything needed to answer independently,
/// never the key or the rationale.
pub fn blind_item(record: &DraftRecord) -> Option<Value> {
    let v = &record.value;
    match record.format {
        Format::Mcq => {
            let options: Vec<Value> = v["options"]
                .as_array()?
                .iter()
                .map(|o| json!({"label": o["label"], "text": o["text"], "anchors": o["anchors"]}))
                .collect();
            Some(json!({"stem": v["stem"], "options": options}))
        }
        Format::CaseVignette => {
            let questions: Vec<Value> = v["questions"]
                .as_array()?
                .iter()
                .map(|q| json!({"question": q["question"]}))
                .collect();
            Some(json!({
                "presentation": v["presentation"],
                "findings": v["findings"],
                "questions": questions,
            }))
        }
        // Tutoring examples carry no answer key — stage 2 does not apply.
        Format::TutoringExample => None,
    }
}

/// The run identity every request body is bound to. The contract and
/// registry versions ride along so a response keyed to superseded rules
/// or sources can never satisfy a current request — the response file's
/// key is the request's hash.
pub struct RequestScope<'a> {
    pub batch_id: &'a str,
    pub contract_version: &'a str,
    pub registry_version: &'a str,
}

/// The fields every request body opens with — kind plus the run/batch
/// identity the response binds to.
fn body_base(scope: &RequestScope, kind: &str, record: &DraftRecord) -> Value {
    json!({
        "kind": kind,
        "batch_id": scope.batch_id,
        "record_id": record.record_id(),
        "format": record.format.slug(),
        "contract_version": scope.contract_version,
    })
}

/// The stage-2 solve request body.
pub fn solve_body(scope: &RequestScope, record: &DraftRecord, item: &Value) -> Value {
    let mut body = body_base(scope, "solve", record);
    body["item"] = item.clone();
    body
}

/// The stage-3/4 verify request body.
pub fn verify_body(
    scope: &RequestScope,
    record: &DraftRecord,
    claims: &[ClaimBlock],
    distractor: Option<&DistractorScreen>,
) -> Value {
    let claims: Vec<Value> = claims
        .iter()
        .map(|c| {
            json!({
                "path": c.path,
                "text": c.text,
                "anchors": c.anchors.iter()
                    .map(|(source_id, locator)| json!({"source_id": source_id, "locator": locator}))
                    .collect::<Vec<Value>>(),
            })
        })
        .collect();
    let distractor = distractor.map(|d| {
        json!({
            "key": d.key,
            "options": d.options.iter()
                .map(|(label, text)| json!({"label": label, "text": text}))
                .collect::<Vec<Value>>(),
            "refutation": d.refutation,
        })
    });
    let mut body = body_base(scope, "verify", record);
    body["registry_version"] = json!(scope.registry_version);
    body["claims"] = json!(claims);
    body["distractor"] = distractor.unwrap_or(Value::Null);
    body
}

/// The stage-5 review request body — the full record as it stands after
/// stages 1–4.
pub fn review_body(scope: &RequestScope, record: &DraftRecord) -> Value {
    let mut body = body_base(scope, "review", record);
    body["record"] = record.value.clone();
    body
}
