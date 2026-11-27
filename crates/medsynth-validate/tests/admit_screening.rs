//! Schema and distractor-screen rejection tests (#9).

mod common;

use common::{Harness, Overrides, gold_shard, sources_path};
use medsynth_validate::admit::Outcome;
use serde_json::{Value, json};

#[test]
fn failed_distractor_screen_rejects_at_stage_4() {
    let mut harness = Harness::new("distractor");
    harness.add_batch_file("mcq.jsonl", &gold_shard("mcq"));
    harness.index();
    let overrides = Overrides {
        distractor: Some("fail".to_owned()),
        ..Overrides::default()
    };
    let opts = harness.opts(harness.config(true), &sources_path());
    let report = harness.settle(&opts, &overrides);

    assert_eq!(report.counts.rejected, report.counts.input);
    for o in &report.outcomes {
        assert_eq!(
            o.stage,
            Some(4),
            "{} rejected at {:?}",
            o.record_id,
            o.stage
        );
        assert_eq!(o.status, "anchor-checked");
    }
}

#[test]
fn schema_defect_rejects_at_stage_1() {
    let mut harness = Harness::new("schema");
    let mut record: Value =
        serde_json::from_str(gold_shard("mcq").lines().next().unwrap()).unwrap();
    record["key"] = json!("Z"); // not a valid option label
    harness.add_batch_file(
        "mcq/bad-0001.json",
        &serde_json::to_string(&record).unwrap(),
    );
    harness.index();
    let opts = harness.opts(harness.config(true), &sources_path());
    let report = harness.settle(&opts, &Overrides::default());

    assert_eq!(report.counts.rejected, 1);
    let o = &report.outcomes[0];
    assert_eq!((o.outcome, o.stage), (Outcome::Rejected, Some(1)));
    assert_eq!(harness.request_count("solve"), 0);
}
