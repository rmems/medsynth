//! Stage 0's lane-specific rights-envelope checks — teacher drafts need a
//! complete, self-consistent manifest envelope; procedural drafts need a
//! `template:` provenance (and a seeded manifest entry when one exists).

use serde_json::Value;

use super::batch::DraftRecord;
use super::eval::{Ctx, Verdict};
use super::hash::{normalize_newlines, sha256_str};

/// Every field a teacher-lane rights envelope must carry, non-blank.
const ENVELOPE_FIELDS: &[&str] = &[
    "record_id",
    "teacher_variant",
    "model_id",
    "pin_id",
    "pin_entry",
    "pin_entry_sha256",
    "content_sha256",
    "route",
    "serving_vendor",
];

pub(super) fn intake_teacher(ctx: &Ctx, record: &DraftRecord) -> Verdict {
    let id = record.record_id();
    let Some(envelope) = ctx.batch.rights_envelope(&id) else {
        return Verdict::reject("teacher-lane draft has no rights envelope in the manifest");
    };
    let missing: Vec<&str> = ENVELOPE_FIELDS
        .iter()
        .copied()
        .filter(|f| envelope[*f].as_str().is_none_or(|s| s.trim().is_empty()))
        .collect();
    if !missing.is_empty() {
        return Verdict::reject(format!(
            "incomplete rights envelope — missing: {}",
            missing.join(", ")
        ));
    }
    if envelope["route"]
        .as_str()
        .is_some_and(|route| route.to_ascii_lowercase().contains("hermes"))
    {
        for field in ["session_file", "session_sha256"] {
            if envelope[field].as_str().is_none_or(|v| v.trim().is_empty()) {
                return Verdict::reject(format!(
                    "Hermes rights envelope missing {field} provenance"
                ));
            }
        }
    }
    if let Some(reason) = envelope_consistent(record, envelope) {
        return Verdict::reject(reason);
    }
    match trusted_pin_consistent(ctx, envelope) {
        Some(reason) => Verdict::reject(reason),
        None => Verdict::Pass,
    }
}

/// The envelope's binding to the record and its own integrity — the first
/// reason it fails, if any.
fn envelope_consistent(record: &DraftRecord, envelope: &Value) -> Option<String> {
    if envelope["record_id"].as_str() != Some(record.record_id().as_str()) {
        return Some("rights envelope names a different record id".to_owned());
    }
    if envelope["content_sha256"].as_str() != Some(record.content_sha256().as_str()) {
        return Some("rights envelope content_sha256 does not match the record".to_owned());
    }
    // The pinned teacher entry must hash to itself — the envelope's own
    // integrity, before any question of which pin the entry names.
    let entry_sha = sha256_str(&normalize_newlines(
        envelope["pin_entry"].as_str().unwrap_or_default(),
    ));
    if entry_sha != envelope["pin_entry_sha256"].as_str().unwrap_or_default() {
        return Some("rights envelope pin_entry does not match pin_entry_sha256".to_owned());
    }
    None
}

/// The envelope's binding to the trusted teacher registry: the pin must be
/// listed there, and the hash the envelope carries must be the trusted entry's
/// hash — not merely the hash of the entry embedded in the envelope itself.
fn trusted_pin_consistent(ctx: &Ctx, envelope: &Value) -> Option<String> {
    let pin_id = envelope["pin_id"].as_str().unwrap_or_default();
    let Some(pin) = ctx.teacher_registry.get(pin_id) else {
        return Some(format!(
            "rights envelope pin_id `{pin_id}` is not listed in the trusted teachers registry"
        ));
    };
    if envelope["pin_entry_sha256"].as_str().unwrap_or_default() != pin.entry_sha256 {
        return Some(format!(
            "rights envelope pin_entry_sha256 does not match the trusted teachers registry entry for `{pin_id}`"
        ));
    }
    None
}

pub(super) fn intake_procedural(record: &DraftRecord, entry: Option<&Value>) -> Verdict {
    if let Some(v) = procedural_provenance(record) {
        return v;
    }
    procedural_entry(entry).unwrap_or(Verdict::Pass)
}

/// The `template:` generator and `template_version` a procedural draft's
/// provenance must carry.
fn procedural_provenance(record: &DraftRecord) -> Option<Verdict> {
    let provenance = &record.value["provenance"];
    let ok = provenance["generator"]
        .as_str()
        .is_some_and(|g| g.starts_with("template:"))
        && provenance["template_version"]
            .as_str()
            .is_some_and(|t| !t.is_empty());
    (!ok).then(|| {
        Verdict::reject("procedural-lane draft lacks its `template:` generator or template_version")
    })
}

/// When a manifest lists the record, the entry must carry the seed and
/// template generator the batch claims produced it.
fn procedural_entry(entry: Option<&Value>) -> Option<Verdict> {
    let entry = entry?;
    let ok = entry["seed"].is_u64() && entry["generator"].as_str().is_some_and(|g| !g.is_empty());
    (!ok).then(|| {
        Verdict::reject("procedural-lane manifest entry lacks its seed or template generator")
    })
}
