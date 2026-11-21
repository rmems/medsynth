//! The approved anchor list handed to a teacher prompt.
//!
//! Every `AnchorPoint` pairs a registry source + locator with the text that
//! source actually backs. Anchors are harvested per claim block — a fact's
//! text with its own anchors — never across a whole record, so a source is
//! never presented as backing a claim it does not anchor.

use serde_json::Value;

use crate::batch::BatchPlan;
use crate::config::TargetCell;
use crate::procedural;
use crate::teacher::AnchorPoint;

/// Build the approved anchor list for a teacher cell.
///
/// Order is deterministic: fact-catalog entries for the cell's subdomain,
/// then per-block anchors from same-subdomain gold records of that format,
/// then one registry locator example per current source covering the
/// subdomain.
pub(crate) fn anchor_pack(plan: &BatchPlan, cell: &TargetCell) -> Vec<AnchorPoint> {
    let mut points = Vec::new();
    fact_points(&mut points, cell);
    gold_points(&mut points, plan, cell);
    registry_points(&mut points, plan, cell);
    points.truncate(16);
    points
}

fn fact_points(points: &mut Vec<AnchorPoint>, cell: &TargetCell) {
    for fact in procedural::FACTS
        .iter()
        .filter(|fact| fact.domain == cell.domain && fact.subdomain == cell.subdomain)
    {
        let text = format!("{} Answer: {}", fact.stem, fact.correct);
        for (source_id, locator) in fact.anchors {
            push_point(points, source_id, locator, &text);
        }
    }
}

fn gold_points(points: &mut Vec<AnchorPoint>, plan: &BatchPlan, cell: &TargetCell) {
    let subdomain = cell.subdomain_slug();
    for gold in plan.gold.records(cell.format) {
        // Anchors must stay inside the cell's subdomain: an anchor from a
        // sibling subdomain would let the teacher cite a source that cannot
        // support the cell's claim.
        if gold.get("subdomain").and_then(Value::as_str) != Some(subdomain) {
            continue;
        }
        collect_block_points(gold, points);
    }
}

fn registry_points(points: &mut Vec<AnchorPoint>, plan: &BatchPlan, cell: &TargetCell) {
    for source in plan.registry.current_covering(cell.subdomain_slug()) {
        if let Some(example) = source.locator_convention.examples.first() {
            push_point(points, &source.source_id, example, "");
        }
    }
}

/// Walk a gold record; for every object carrying a `text` field, pair the
/// anchors inside that object's subtree with that object's own text.
fn collect_block_points(value: &Value, points: &mut Vec<AnchorPoint>) {
    match value {
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                let mut anchors = Vec::new();
                collect_anchors(value, &mut anchors);
                let fact = truncate(text, 240);
                for (source_id, locator) in anchors {
                    push_point(points, &source_id, &locator, &fact);
                }
            }
            for child in object.values() {
                collect_block_points(child, points);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_block_points(item, points);
            }
        }
        _ => {}
    }
}

/// Walk a JSON tree and collect every `(source_id, locator)` anchor object.
///
/// Public so the sample-batch test can verify committed drafts against the
/// same walker the pipeline uses — the two must never diverge.
pub fn collect_anchors(value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(object) => {
            if let (Some(source_id), Some(locator)) = (
                object.get("source_id").and_then(Value::as_str),
                object.get("locator").and_then(Value::as_str),
            ) {
                out.push((source_id.to_owned(), locator.to_owned()));
            }
            for child in object.values() {
                collect_anchors(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_anchors(item, out);
            }
        }
        _ => {}
    }
}

fn push_point(points: &mut Vec<AnchorPoint>, source_id: &str, locator: &str, fact: &str) {
    if points
        .iter()
        .any(|point| point.source_id == source_id && point.locator == locator)
    {
        return;
    }
    points.push(AnchorPoint {
        source_id: source_id.to_owned(),
        locator: locator.to_owned(),
        fact: fact.to_owned(),
    });
}

fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}
