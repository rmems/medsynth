//! Stages 0–1: the rights-envelope intake gate and the shared schema
//! check. Both are deterministic — no fulfillment traffic.

use serde_json::Value;

use crate::{RecordFile, run_schema_check};

use super::batch::DraftRecord;
use super::eval::{Ctx, Verdict};
use super::hash::{normalize_newlines, sha256_str};

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
    let id = record.record_id();
    let entry = ctx.batch.record_entry(&id);
    if ctx.batch.has_manifest() && entry.is_none() {
        return Verdict::reject("record is not listed in the batch manifest");
    }
    if let Some(entry) = entry {
        // The manifest's content hash is what makes the listing mean
        // anything — an entry without one cannot be integrity-checked.
        let valid_sha = entry["sha256"]
            .as_str()
            .is_some_and(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()));
        if !valid_sha {
            return Verdict::reject("manifest entry lacks a valid content sha256");
        }
        if entry["sha256"].as_str() != Some(record.content_sha256().as_str()) {
            return Verdict::reject("record content diverges from the manifest's sha256");
        }
    }
    let declared = entry.and_then(|e| e["lane"].as_str());
    let inferred = lane_of(record);
    if let Some(lane) = declared
        && lane != inferred
    {
        return Verdict::reject(format!(
            "manifest lane `{lane}` disagrees with the record's provenance (`{inferred}`)"
        ));
    }
    match declared.unwrap_or(&inferred) {
        "teacher" => intake_teacher(ctx, record),
        "procedural" => intake_procedural(record, entry),
        "agent" => Verdict::Pass,
        other => Verdict::reject(format!("manifest lane `{other}` is not a drafting lane")),
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

fn intake_teacher(ctx: &Ctx, record: &DraftRecord) -> Verdict {
    let id = record.record_id();
    let Some(envelope) = ctx.batch.rights_envelope(&id) else {
        return Verdict::reject("teacher-lane draft has no rights envelope in the manifest");
    };
    let mut missing = Vec::new();
    for field in [
        "record_id",
        "teacher_variant",
        "model_id",
        "pin_id",
        "pin_entry",
        "pin_entry_sha256",
        "content_sha256",
    ] {
        if envelope[field].as_str().is_none_or(|s| s.trim().is_empty()) {
            missing.push(field);
        }
    }
    if !missing.is_empty() {
        return Verdict::reject(format!(
            "incomplete rights envelope — missing: {}",
            missing.join(", ")
        ));
    }
    if envelope["record_id"].as_str() != Some(id.as_str()) {
        return Verdict::reject("rights envelope names a different record id");
    }
    if envelope["content_sha256"].as_str() != Some(record.content_sha256().as_str()) {
        return Verdict::reject("rights envelope content_sha256 does not match the record");
    }
    // The pinned teacher entry must hash to itself — the envelope's own
    // integrity, before any question of which pin the entry names.
    if sha256_str(&normalize_newlines(
        envelope["pin_entry"].as_str().unwrap_or_default(),
    )) != envelope["pin_entry_sha256"].as_str().unwrap_or_default()
    {
        return Verdict::reject("rights envelope pin_entry does not match pin_entry_sha256");
    }
    Verdict::Pass
}

fn intake_procedural(record: &DraftRecord, entry: Option<&Value>) -> Verdict {
    let provenance = &record.value["provenance"];
    if !provenance["generator"]
        .as_str()
        .is_some_and(|g| g.starts_with("template:"))
        || provenance["template_version"]
            .as_str()
            .is_none_or(|t| t.is_empty())
    {
        return Verdict::reject(
            "procedural-lane draft lacks its `template:` generator or template_version",
        );
    }
    if let Some(entry) = entry
        && (!entry["seed"].is_u64() || entry["generator"].as_str().is_none_or(|g| g.is_empty()))
    {
        return Verdict::reject(
            "procedural-lane manifest entry lacks its seed or template generator",
        );
    }
    Verdict::Pass
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
