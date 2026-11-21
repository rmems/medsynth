use std::collections::{BTreeSet, HashSet};

use medsynth_schema::VerificationStatus;

use crate::{Check, Finding, RecordFile};

/// Rank of a [`VerificationStatus`] in the provenance ordering —
/// `unverified` < `key-checked` < `anchor-checked` < `admitted`, the
/// declaration order of the `medsynth-schema` enum.
fn status_rank(value: &serde_json::Value) -> Option<u8> {
    let s = value.get("verification_status")?.as_str()?;
    [
        VerificationStatus::Unverified,
        VerificationStatus::KeyChecked,
        VerificationStatus::AnchorChecked,
        VerificationStatus::Admitted,
    ]
    .iter()
    .position(|v| {
        serde_json::to_value(v)
            .ok()
            .and_then(|j| j.as_str().map(str::to_owned))
            .as_deref()
            == Some(s)
    })
    .map(|p| p as u8)
}

/// `anchor-presence` — a record above `unverified` must carry at least one
/// anchor citing a source id in the approved registry (contract §4).
pub(super) fn anchor_presence_check(
    record: &RecordFile,
    registry: &HashSet<String>,
    findings: &mut Vec<Finding>,
) {
    let Some(rank) = status_rank(&record.value) else {
        return; // missing/garbage status is the schema check's finding
    };
    // Every citation must be into the registry (contract §4) at any
    // status — a listed anchor must not mask unlisted ones.
    let mut unlisted = BTreeSet::new();
    collect_unlisted(&record.value, registry, &mut unlisted);
    for source_id in unlisted {
        findings.push(Finding {
            path: record.path.clone(),
            line: record.line,
            record_id: record.record_id(),
            check: Check::AnchorPresence,
            detail: format!("cites source id `{source_id}`, which is not in the approved registry"),
        });
    }
    if rank == 0 {
        return;
    }
    if count_registry_anchors(&record.value, registry) == 0 {
        findings.push(Finding {
            path: record.path.clone(),
            line: record.line,
            record_id: record.record_id(),
            check: Check::AnchorPresence,
            detail: format!(
                "verification_status `{}` is above unverified but the record \
                 carries no anchors into the approved source registry",
                record.value["verification_status"].as_str().unwrap_or("?")
            ),
        });
    }
}

/// Collect every cited `source_id` not present in the registry.
fn collect_unlisted(
    value: &serde_json::Value,
    registry: &HashSet<String>,
    out: &mut BTreeSet<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                if k == "anchors" {
                    collect_unlisted_anchors(v, registry, out);
                } else {
                    collect_unlisted(v, registry, out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for i in items {
                collect_unlisted(i, registry, out);
            }
        }
        _ => {}
    }
}

fn collect_unlisted_anchors(
    value: &serde_json::Value,
    registry: &HashSet<String>,
    out: &mut BTreeSet<String>,
) {
    let Some(anchors) = value.as_array() else {
        return;
    };
    for a in anchors {
        if let Some(id) = a["source_id"].as_str()
            && !registry.contains(id)
        {
            out.insert(id.to_owned());
        }
    }
}

/// Count anchor objects anywhere in the record whose `source_id` is in the
/// approved registry — an anchor to an unlisted source is no anchor
/// (contract §4).
fn count_registry_anchors(value: &serde_json::Value, registry: &HashSet<String>) -> usize {
    match value {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                if k == "anchors" {
                    v.as_array().map_or(0, |anchors| {
                        anchors
                            .iter()
                            .filter(|a| {
                                a["source_id"]
                                    .as_str()
                                    .is_some_and(|id| registry.contains(id))
                            })
                            .count()
                    })
                } else {
                    count_registry_anchors(v, registry)
                }
            })
            .sum(),
        serde_json::Value::Array(items) => items
            .iter()
            .map(|i| count_registry_anchors(i, registry))
            .sum(),
        _ => 0,
    }
}
