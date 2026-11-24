//! Scanning the published corpus — the record ids stage 1's duplicate
//! and re-admission checks compare against.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Ids already published under `data/` — every record file the CI gate
/// scans (all `.jsonl` shards and `.json` records, nested dirs included),
/// minus shards named for this batch id so a re-run is idempotent rather
/// than self-conflicting. Malformed files are hard errors: a gate that
/// cannot read the corpus cannot claim uniqueness.
pub(super) fn published_record_ids(
    data_dir: &Path,
    batch_id: &str,
) -> Result<HashSet<String>, String> {
    let own_shard = format!("{batch_id}.jsonl");
    let mut ids = HashSet::new();
    let mut files = Vec::new();
    walk_tree(data_dir, &mut files)?;
    for path in files {
        if path.file_name().and_then(|n| n.to_str()) == Some(own_shard.as_str()) {
            continue;
        }
        let rel = path.strip_prefix(data_dir).unwrap_or(&path).to_path_buf();
        if super::batch::is_sidecar(&rel) {
            continue;
        }
        collect_file_ids(&path, &mut ids)?;
    }
    Ok(ids)
}

/// Parse one published file — each `.jsonl` line, or the single record a
/// `.json` file holds — into the id set.
fn collect_file_ids(path: &Path, ids: &mut HashSet<String>) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        let v: Value = serde_json::from_str(&text)
            .map_err(|e| format!("{}: unparseable published record: {e}", path.display()))?;
        if let Some(id) = v["id"].as_str() {
            ids.insert(id.to_owned());
        }
        return Ok(());
    }
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).map_err(|e| {
            format!(
                "{}:{}: unparseable published record: {e}",
                path.display(),
                i + 1
            )
        })?;
        if let Some(id) = v["id"].as_str() {
            ids.insert(id.to_owned());
        }
    }
    Ok(())
}

/// Every `.jsonl`/`.json` file under `dir`, recursing — a directory that
/// cannot be read is a hard error, never an empty answer.
fn walk_tree(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("cannot read {}: {e}", dir.display())),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("cannot stat {}: {e}", path.display()))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            walk_tree(&path, out)?;
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("json") | Some("jsonl")
        ) {
            out.push(path);
        }
    }
    Ok(())
}
