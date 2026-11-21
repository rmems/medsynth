use std::collections::BTreeSet;

use crate::*;

/// Run every check over the corpus and return all findings. `registry` is
/// the approved source-id set from [`load_registry`].
pub fn run_checks(
    records: &[RecordFile],
    validators: &Validators,
    registry: &HashSet<String>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    duplicate_id_check(records, &mut findings);
    for record in records {
        schema_check(record, validators, &mut findings);
        key_rationale_check(record, &mut findings);
        anchor_presence_check(record, registry, &mut findings);
    }
    originality::screen(records, &mut findings);
    findings.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.check.name().cmp(b.check.name()))
            .then(a.detail.cmp(&b.detail))
    });
    findings
}

/// Record ids must be unique across the corpus — a duplicate makes the
/// identity ambiguous for consumers and for finding reports.
fn duplicate_id_check(records: &[RecordFile], findings: &mut Vec<Finding>) {
    let mut seen: HashMap<String, &RecordFile> = HashMap::new();
    for record in records {
        let id = record.record_id();
        if id == "<no-id>" {
            continue;
        }
        if let Some(first) = seen.get(&id) {
            findings.push(Finding {
                path: record.path.clone(),
                line: record.line,
                record_id: id.clone(),
                check: Check::Schema,
                detail: format!(
                    "duplicate record id (first seen at {})",
                    first.path.display()
                ),
            });
        } else {
            seen.insert(id, record);
        }
    }
}

/// `schema` — JSON Schema validation plus deserialize (of the raw record
/// text, so duplicate fields fail) + cross-field `validate()`.
fn schema_check(record: &RecordFile, validators: &Validators, findings: &mut Vec<Finding>) {
    let mut push = |detail: String| {
        findings.push(Finding {
            path: record.path.clone(),
            line: record.line,
            record_id: record.record_id(),
            check: Check::Schema,
            detail,
        });
    };
    let Some(validator) = validators.by_format.get(&record.format) else {
        push(format!("no schema for format {}", record.format.slug()));
        return;
    };
    let schema_errors: Vec<String> = validator
        .iter_errors(&record.value)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    if !schema_errors.is_empty() {
        for detail in schema_errors {
            push(detail);
        }
        return;
    }
    // Deserialization failure names the serde path; a validate() failure
    // lists the cross-field rules broken.
    match record.format {
        Format::Mcq => cross_detail::<McqRecord>(record, findings),
        Format::CaseVignette => cross_detail::<CaseVignetteRecord>(record, findings),
        Format::TutoringExample => cross_detail::<TutoringExampleRecord>(record, findings),
    }
}

fn cross_detail<T>(record: &RecordFile, findings: &mut Vec<Finding>)
where
    T: serde::de::DeserializeOwned + CrossValidate,
{
    let mut push = |detail: String| {
        findings.push(Finding {
            path: record.path.clone(),
            line: record.line,
            record_id: record.record_id(),
            check: Check::Schema,
            detail,
        });
    };
    match serde_json::from_str::<T>(&record.raw) {
        Err(e) => push(format!("record does not deserialize: {e}")),
        Ok(r) => {
            if let Err(errors) = r.validate() {
                for e in errors {
                    push(format!("cross-field rule: {e}"));
                }
            }
        }
    }
}

/// Lets `schema_check` call each record type's `validate` generically.
pub trait CrossValidate {
    /// Cross-field `validate` from `medsynth-schema`.
    fn validate(&self) -> Result<(), Vec<ValidationError>>;
}
impl CrossValidate for McqRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        McqRecord::validate(self)
    }
}
impl CrossValidate for CaseVignetteRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        CaseVignetteRecord::validate(self)
    }
}
impl CrossValidate for TutoringExampleRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        TutoringExampleRecord::validate(self)
    }
}

/// `key-rationale` — rationale present, non-empty, referencing the keyed
/// answer; answer fields non-empty; `AnchoredClaim` anchors non-empty.
fn key_rationale_check(record: &RecordFile, findings: &mut Vec<Finding>) {
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
    for (kind, claims) in [("questions", "answer"), ("teaching_points", "")] {
        let Some(items) = v.get(kind).and_then(|q| q.as_array()) else {
            continue;
        };
        for (i, item) in items.iter().enumerate() {
            let claim = if claims.is_empty() {
                item
            } else {
                &item[claims]
            };
            if claim["text"].as_str().is_none_or(|t| t.trim().is_empty()) {
                let label = if claims.is_empty() {
                    format!("{kind}[{i}].text is empty")
                } else {
                    format!("{kind}[{i}].{claims}.text is empty")
                };
                push_lint(record, findings, label);
            }
            if claim["anchors"].as_array().is_none_or(|a| a.is_empty()) {
                let label = if claims.is_empty() {
                    format!("{kind}[{i}] has no anchors")
                } else {
                    format!("{kind}[{i}].{claims} has no anchors")
                };
                push_lint(record, findings, label);
            }
        }
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
fn anchor_presence_check(
    record: &RecordFile,
    registry: &HashSet<String>,
    findings: &mut Vec<Finding>,
) {
    let Some(rank) = status_rank(&record.value) else {
        return; // missing/garbage status is the schema check's finding
    };
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
    // Every citation must be into the registry (contract §4) — a listed
    // anchor must not mask unlisted ones.
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
                    if let Some(anchors) = v.as_array() {
                        for a in anchors {
                            if let Some(id) = a["source_id"].as_str()
                                && !registry.contains(id)
                            {
                                out.insert(id.to_owned());
                            }
                        }
                    }
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
