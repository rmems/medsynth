//! Stages 0–1: the rights-envelope intake gate and the shared schema
//! check. Both are deterministic — no fulfillment traffic.

use serde_json::Value;

use crate::{RecordFile, run_schema_check};

use super::batch::DraftRecord;
use super::envelope;
use super::eval::{Ctx, Verdict};

/// Stage 0 — draft status, manifest membership and content hash, then
/// the lane-specific envelope checks.
pub(super) fn stage_intake(ctx: &Ctx, record: &DraftRecord) -> Verdict {
    if record.value["verification_status"].as_str() != Some("unverified") {
        return Verdict::reject(format!(
            "verification_status is `{}`, not `unverified` — only drafts enter admission",
            record.value["verification_status"]
                .as_str()
                .unwrap_or("<missing>")
        ));
    }
    let entry = ctx.batch.record_entry(&record.record_id());
    if let Some(v) = entry_integrity(ctx, record, entry) {
        return v;
    }
    match resolve_lane(record, entry) {
        Ok(lane) => match lane.as_str() {
            "teacher" => envelope::intake_teacher(ctx, record),
            "procedural" => envelope::intake_procedural(record, entry),
            _ => Verdict::Pass,
        },
        Err(v) => v,
    }
}

/// The manifest listing's integrity: membership when a manifest exists,
/// then a valid content hash matching the record as written.
fn entry_integrity(ctx: &Ctx, record: &DraftRecord, entry: Option<&Value>) -> Option<Verdict> {
    if ctx.batch.has_manifest() && entry.is_none() {
        return Some(Verdict::reject(
            "record is not listed in the batch manifest",
        ));
    }
    let entry = entry?;
    // The manifest's content hash is what makes the listing mean
    // anything — an entry without one cannot be integrity-checked.
    let sha = entry["sha256"].as_str();
    if !sha.is_some_and(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())) {
        return Some(Verdict::reject(
            "manifest entry lacks a valid content sha256",
        ));
    }
    if sha != Some(record.content_sha256().as_str()) {
        return Some(Verdict::reject(
            "record content diverges from the manifest's sha256",
        ));
    }
    None
}

/// The drafting lane: manifest-declared when present (and consistent with
/// provenance), else the provenance-inferred lane. Unknown or
/// contradictory lanes reject.
fn resolve_lane(record: &DraftRecord, entry: Option<&Value>) -> Result<String, Verdict> {
    if let Some(e) = entry
        && e.get("lane")
            .is_some_and(|l| !l.is_null() && !l.is_string())
    {
        return Err(Verdict::reject(
            "manifest entry's `lane` is not a string — the listing is malformed",
        ));
    }
    let declared = entry.and_then(|e| e["lane"].as_str());
    let inferred = lane_of(record);
    if let Some(lane) = declared
        && lane != inferred
    {
        return Err(Verdict::reject(format!(
            "manifest lane `{lane}` disagrees with the record's provenance (`{inferred}`)"
        )));
    }
    match declared.unwrap_or(&inferred) {
        lane @ ("teacher" | "procedural" | "agent") => Ok(lane.to_owned()),
        other => Err(Verdict::reject(format!(
            "manifest lane `{other}` is not a drafting lane"
        ))),
    }
}

/// The drafting lane inferred from `provenance` — `template:` generators
/// are procedural, a recorded `model` makes the draft teacher-lane,
/// anything else is agent-drafted and carries no envelope.
fn lane_of(record: &DraftRecord) -> String {
    let provenance = &record.value["provenance"];
    if provenance["generator"]
        .as_str()
        .is_some_and(|g| g.starts_with("template:"))
    {
        return "procedural".to_owned();
    }
    if provenance["model"].as_str().is_some_and(|m| !m.is_empty()) {
        return "teacher".to_owned();
    }
    "agent".to_owned()
}

/// Stage 1 — the shared schema check plus admission-level identity rules:
/// no duplicate id within the batch, none already admitted under `data/`.
pub(super) fn stage_schema(ctx: &Ctx, record: &DraftRecord) -> Verdict {
    let mut reasons: Vec<String> = Vec::new();
    let record_file = RecordFile {
        path: record.path.clone(),
        line: record.line,
        format: record.format,
        value: record.value.clone(),
        raw: record.raw.clone(),
    };
    for finding in run_schema_check(&record_file, ctx.validators) {
        reasons.push(finding.detail);
    }
    let id = record.record_id();
    if ctx.id_counts.get(&id).copied().unwrap_or(0) > 1 {
        reasons.push("duplicate record id within the batch".to_owned());
    }
    if ctx.admitted_ids.contains(&id) {
        reasons.push("record id is already admitted under data/".to_owned());
    }
    if reasons.is_empty() {
        Verdict::Pass
    } else {
        Verdict::Reject(reasons)
    }
}
