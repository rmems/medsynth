use std::collections::BTreeSet;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::Format;
use crate::originality::record_text;

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

pub(crate) fn prompt_text(value: &serde_json::Value, format: Format) -> String {
    let path: &[&str] = match format {
        Format::Mcq => &["stem", "text"],
        Format::CaseVignette => &["presentation"],
        Format::TutoringExample => &["learner_prompt", "text"],
    };
    let mut current = value;
    for key in path {
        current = &current[key];
    }
    current
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| record_text(value))
}

pub(crate) fn collect_anchors(value: &serde_json::Value, out: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if key == "anchors" {
                    if let serde_json::Value::Array(items) = child {
                        for anchor in items {
                            if let (Some(source), Some(locator)) = (
                                anchor.get("source_id").and_then(|v| v.as_str()),
                                anchor.get("locator").and_then(|v| v.as_str()),
                            ) {
                                out.insert(format!("{source}::{locator}"));
                            }
                        }
                    }
                } else {
                    collect_anchors(child, out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_anchors(item, out);
            }
        }
        _ => {}
    }
}
