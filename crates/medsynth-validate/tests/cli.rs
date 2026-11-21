//! End-to-end tests of the validator binary: exit code and report output
//! on a clean corpus, a near-miss corpus, a `.jsonl` shard, and an empty
//! corpus.

use std::path::{Path, PathBuf};
use std::process::Command;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .to_path_buf()
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_medsynth-validate"))
}

/// A scratch corpus rooted in a unique temp dir.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "medsynth-validate-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn write(&self, rel: &str, contents: &str) {
        let path = self.0.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn fixture_text(format: &str, name: &str) -> String {
    let path = workspace_root().join("fixtures").join(format).join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

#[test]
fn exits_zero_on_the_clean_repo() {
    let out = bin().arg(workspace_root()).output().expect("binary runs");
    assert!(
        out.status.success(),
        "clean repo must pass: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn exits_nonzero_and_names_checks_on_near_misses() {
    let scratch = Scratch::new("near-miss");

    // Wrong key: rationale explains A, key points at B.
    let good: serde_json::Value =
        serde_json::from_str(&fixture_text("mcq", "valid-enzyme-kinetics.json")).unwrap();
    let mut wrong_key = good.clone();
    wrong_key["id"] = serde_json::json!("mcq-nearmiss-0001");
    wrong_key["key"] = serde_json::json!("B");
    scratch.write(
        "data/mcq/wrong-key.json",
        &serde_json::to_string_pretty(&wrong_key).unwrap(),
    );

    // Missing anchor: key-checked record with every anchor removed.
    let mut no_anchors = good.clone();
    no_anchors["id"] = serde_json::json!("mcq-nearmiss-0002");
    no_anchors["verification_status"] = serde_json::json!("anchor-checked");
    fn strip(v: &mut serde_json::Value) {
        match v {
            serde_json::Value::Object(map) => {
                map.retain(|k, _| k != "anchors");
                for (_, v) in map.iter_mut() {
                    strip(v);
                }
            }
            serde_json::Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut no_anchors);
    scratch.write(
        "data/mcq/missing-anchor.json",
        &serde_json::to_string_pretty(&no_anchors).unwrap(),
    );

    // The registry must exist for the anchor-presence check.
    scratch.write(
        "docs/sources.json",
        &std::fs::read_to_string(workspace_root().join("docs/sources.json")).unwrap(),
    );

    let out = bin().arg(&scratch.0).output().expect("binary runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "near-miss corpus must fail");
    assert!(stderr.contains("[key-rationale]"), "stderr: {stderr}");
    assert!(stderr.contains("[anchor-presence]"), "stderr: {stderr}");
    assert!(stderr.contains("mcq-nearmiss-0001"), "stderr: {stderr}");
    assert!(stderr.contains("mcq-nearmiss-0002"), "stderr: {stderr}");
}

#[test]
fn validates_jsonl_shards_line_by_line() {
    let scratch = Scratch::new("jsonl");
    let good = fixture_text("mcq", "valid-enzyme-kinetics.json");
    let mut bad: serde_json::Value = serde_json::from_str(&good).unwrap();
    bad["id"] = serde_json::json!("mcq-jsonl-0002");
    bad["key"] = serde_json::json!("B");
    let shard = format!(
        "{}\n{}\n",
        serde_json::to_string(&serde_json::from_str::<serde_json::Value>(&good).unwrap()).unwrap(),
        serde_json::to_string(&bad).unwrap()
    );
    scratch.write("data/mcq/batch.jsonl", &shard);
    scratch.write(
        "docs/sources.json",
        &std::fs::read_to_string(workspace_root().join("docs/sources.json")).unwrap(),
    );

    let out = bin().arg(&scratch.0).output().expect("binary runs");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "bad jsonl line must fail");
    assert!(
        stderr.contains("batch.jsonl:2"),
        "finding must name the shard line, stderr: {stderr}"
    );
    assert!(stderr.contains("[key-rationale]"), "stderr: {stderr}");
}

#[test]
fn fails_when_the_corpus_is_empty() {
    let scratch = Scratch::new("empty");
    let out = bin().arg(&scratch.0).output().expect("binary runs");
    assert!(
        !out.status.success(),
        "an empty corpus must not pass silently"
    );
}

#[test]
fn gold_discovery_excludes_only_probes_and_review_metadata() {
    let scratch = Scratch::new("gold-discovery");
    let good = fixture_text("mcq", "valid-enzyme-kinetics.json");
    scratch.write(
        "data/gold/mcq.jsonl",
        &format!(
            "{}\n",
            serde_json::to_string(&serde_json::from_str::<serde_json::Value>(&good).unwrap())
                .unwrap()
        ),
    );
    scratch.write("data/gold/review-manifest.json", "{\"records\": []}");
    scratch.write("data/gold/near-miss/manifest.json", "{\"records\": []}");
    scratch.write(
        "data/gold/near-miss/mcq/probe.json",
        "not a positive record",
    );
    scratch.write(
        "docs/sources.json",
        &std::fs::read_to_string(workspace_root().join("docs/sources.json")).unwrap(),
    );
    let out = bin().arg(&scratch.0).output().expect("binary runs");
    assert!(
        out.status.success(),
        "flat gold shard loads without probes/metadata: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    scratch.write("data/gold/unexpected.json", "{\"not\": \"a record\"}");
    let out = bin().arg(&scratch.0).output().expect("binary runs");
    assert!(
        !out.status.success(),
        "unknown data must not silently disappear"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("unexpected.json"));
}

#[test]
fn flat_format_shards_are_scoped_to_gold() {
    use medsynth_validate::{Format, format_of};
    for format in Format::all() {
        assert_eq!(
            format_of(&PathBuf::from(format!("data/gold/{}.jsonl", format.slug()))),
            Some(format)
        );
        for parent in ["data", "fixtures", "other/gold"] {
            assert_eq!(
                format_of(&PathBuf::from(format!("{parent}/{}.jsonl", format.slug()))),
                None
            );
        }
    }
    assert_eq!(format_of(Path::new("data/gold/mcq.json")), None);
}
