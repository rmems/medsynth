//! Startup configuration and registry fail-closed tests (#9).

mod common;

use std::fs;

use common::{Harness, Overrides, gold_shard, sources_path, workspace_root};
use medsynth_validate::admit;
use serde_json::{Value, json};

#[test]
fn missing_or_unparseable_teacher_registry_fails_closed() {
    for (name, contents) in [
        ("missing-teachers", None),
        ("unparseable-teachers", Some("# no pins here\n")),
        ("blank-pin-teachers", Some("### Pin: `blank`\n")),
        (
            "duplicate-pin-teachers",
            Some(
                "### Pin: `one`\n\n- **pin_id:** `same`\n\n### Pin: `two`\n\n- **pin_id:** `same`\n",
            ),
        ),
    ] {
        let mut harness = Harness::new(name);
        harness.add_batch_file("mcq.jsonl", &gold_shard("mcq"));
        harness.index();
        let teachers = harness.dir.join("teachers.md");
        if let Some(contents) = contents {
            fs::write(&teachers, contents).unwrap();
        }
        let opts = harness.opts_with_teachers(harness.config(true), &sources_path(), &teachers);
        let err = admit::run(&opts).expect_err("teacher registry failure must refuse the run");
        assert!(
            err.contains("teacher pin registry"),
            "unexpected error for {name}: {err}"
        );
        assert!(!harness.dir.join("admission-log.json").exists());
        assert_eq!(harness.request_count("solve"), 0);
    }
}

/// `load_run_config` parses a good file and names the reason each
/// malformed variant fails: unreadable, unparseable, blank or stale
/// contract version, out-of-range sample rate, incomplete pins, and
/// colliding run ids.
#[test]
fn run_config_load_validates_shape() {
    let harness = Harness::new("config-shape");
    let path = harness.dir.join("run-config.json");
    let write = |body: &str| {
        fs::write(&path, body).unwrap();
        admit::load_run_config(&path)
    };
    let good = json!({
        "fulfillment_dir": "fulfill",
        "contract_version": "0.1",
        "solver": {"run_id": "s1", "model_id": "m1"},
        "verifier": {"run_id": "v1"},
        "reviewer": {"run_id": "r1"},
    });
    let config = write(&good.to_string()).expect("valid config");
    assert_eq!(config.contract_version, "0.1");
    assert_eq!(config.sample_rate, 0.10); // default

    // Unreadable vs unparseable are distinct diagnostics.
    let missing = admit::load_run_config(&harness.dir.join("no-such-config.json"));
    assert!(missing.unwrap_err().contains("cannot read"));
    assert!(write("{nope").unwrap_err().contains("run config"));

    let mut v = good.clone();
    // Blank, padded, and stale versions all fail the exact-match check —
    // the string is stamped verbatim, so no trimming tolerance.
    for bad in [" ", " 0.1 ", "0.0"] {
        v["contract_version"] = json!(bad);
        assert!(
            write(&v.to_string())
                .unwrap_err()
                .contains("not the gate's"),
            "contract_version `{bad}` must fail exact match"
        );
    }
    v = good.clone();
    v["sample_rate"] = json!(1.5);
    assert!(write(&v.to_string()).unwrap_err().contains("sample_rate"));

    v = good.clone();
    v["solver"] = json!({"run_id": " ", "model_id": "m"});
    assert!(write(&v.to_string()).unwrap_err().contains("solver pin"));

    v = good.clone();
    v["verifier"] = json!({"run_id": " "});
    assert!(write(&v.to_string()).unwrap_err().contains("verifier"));

    // A present-but-blank optional model id is a malformed pin, not an
    // unpinned model.
    v = good.clone();
    v["verifier"] = json!({"run_id": "v1", "model_id": "  "});
    assert!(write(&v.to_string()).unwrap_err().contains("model_id"));
    v = good.clone();
    v["reviewer"] = json!({"run_id": "r1", "model_id": "	"});
    assert!(write(&v.to_string()).unwrap_err().contains("model_id"));

    v = good.clone();
    v["reviewer"] = json!({"run_id": "s1"});
    assert!(write(&v.to_string()).unwrap_err().contains("distinct"));
}

#[test]
fn unpinned_solver_refuses_to_start() {
    let mut harness = Harness::new("unpinned");
    harness.add_batch_file("mcq.jsonl", &gold_shard("mcq"));
    harness.index();
    // Same shape the loader rejects: no solver pin over keyed records.
    let opts = harness.opts(harness.config(false), &sources_path());
    let err = admit::run(&opts).expect_err("unpinned solver must refuse");
    assert!(err.contains("solver"), "unexpected error: {err}");
    // And the whole batch produced no output.
    assert!(!harness.dir.join("admission-log.json").exists());
}

#[test]
fn stale_registry_entry_holds_for_reanchoring() {
    // A registry entry that changed since drafting holds its records —
    // never a rejection, per contract §8/§7.
    let mut harness = Harness::new("stale-registry");
    let batch_src = workspace_root().join("drafts/sample-batch");
    for entry in fs::read_dir(&batch_src).unwrap().flatten() {
        let path = entry.path();
        if path.is_file()
            && matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("json") | Some("jsonl")
            )
        {
            fs::copy(&path, harness.dir.join("batch").join(entry.file_name())).unwrap();
        }
    }
    harness.index();
    // Drift every registry entry: the receipts' pinned hashes no longer
    // match anything current.
    let mut sources: Value =
        serde_json::from_str(&fs::read_to_string(sources_path()).unwrap()).unwrap();
    for entry in sources["sources"].as_array_mut().unwrap() {
        entry["year"] = json!(1900);
    }
    let drifted = harness.dir.join("sources-drifted.json");
    fs::write(&drifted, serde_json::to_string_pretty(&sources).unwrap()).unwrap();

    let opts = harness.opts(harness.config(true), &drifted);
    let report = harness.settle(&opts, &Overrides::default());

    assert_eq!(
        (
            report.counts.rejected,
            report.counts.admitted,
            report.counts.held,
        ),
        (0, 0, report.counts.input)
    );
    assert!(report.held_by_stage.contains_key("3-anchor"));
    for o in &report.outcomes {
        assert_eq!(o.stage, Some(3), "{} held at {:?}", o.record_id, o.stage);
        assert!(
            o.reasons
                .iter()
                .any(|r| r.contains("changed since drafting")),
            "{} missing stale-registry hold: {:?}",
            o.record_id,
            o.reasons
        );
    }
}
