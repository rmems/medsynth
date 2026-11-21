use std::collections::{HashMap, HashSet};

use crate::{Check, Finding, RecordFile, Validators, originality};

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
mod anchors;
mod key_rationale;
mod schema;

use anchors::anchor_presence_check;
use key_rationale::key_rationale_check;
use schema::schema_check;
