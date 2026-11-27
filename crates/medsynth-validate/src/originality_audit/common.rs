use std::collections::BTreeSet;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::Format;

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn sha256_text(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    sha256_bytes(normalized.as_bytes())
}

pub(crate) fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub(crate) fn prompt_text(value: &serde_json::Value, format: Format) -> Option<String> {
    let path: &[&str] = match format {
        Format::Mcq => &["stem", "text"],
        Format::CaseVignette => &["presentation"],
        Format::TutoringExample => &["learner_prompt", "text"],
    };
    let mut current = value;
    for key in path {
        current = &current[key];
    }
    current.as_str().map(str::to_owned)
}

fn anchor_key(anchor: &serde_json::Value) -> Option<String> {
    let source = anchor.get("source_id")?.as_str()?;
    let locator = anchor.get("locator")?.as_str()?;
    Some(format!("{source}::{locator}"))
}

fn collect_anchor_array(items: &[serde_json::Value], out: &mut BTreeSet<String>) {
    out.extend(items.iter().filter_map(anchor_key));
}

fn collect_object_anchors(
    map: &serde_json::Map<String, serde_json::Value>,
    out: &mut BTreeSet<String>,
) {
    for (key, child) in map {
        if key == "anchors" {
            if let serde_json::Value::Array(items) = child {
                collect_anchor_array(items, out);
            }
        } else {
            collect_anchors(child, out);
        }
    }
}

pub(crate) fn collect_anchors(value: &serde_json::Value, out: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => collect_object_anchors(map, out),
        serde_json::Value::Array(items) => {
            for item in items {
                collect_anchors(item, out);
            }
        }
        _ => {}
    }
}
