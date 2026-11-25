//! Scanning the published corpus — the record ids stage 1's duplicate
//! and re-admission checks compare against.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Ids already published under `data/` — every record file the CI gate
/// scans (all `.jsonl` shards and `.json` records, nested dirs included),
/// minus this run's own shards so a re-run is idempotent rather than
/// self-conflicting, and minus the gold near-miss probes: they are
/// corrective fixtures CI itself excludes, not published records, so a
/// corrected probe reusing its id must not block itself at the duplicate
/// check. Malformed files are hard errors: a gate that cannot read the
/// corpus cannot claim uniqueness.
pub(super) fn published_record_ids(
    data_dir: &Path,
    batch_id: &str,
) -> Result<HashSet<String>, String> {
    // This run's own shards are exactly `data/<format>/<batch_id>.jsonl`
    // — comparing full relative paths, not file names, so a nested shard
    // that happens to share the name (`mcq/archive/<batch>.jsonl`) still
    // counts toward the duplicate scan.
    let own_shards: Vec<std::path::PathBuf> = crate::Format::all()
        .iter()
        .map(|f| std::path::PathBuf::from(f.slug()).join(format!("{batch_id}.jsonl")))
        .collect();
    let mut ids = HashSet::new();
    let mut files = Vec::new();
    walk_tree(data_dir, &mut files)?;
    let near_miss = std::path::PathBuf::from("gold").join("near-miss");
    for path in files {
        let rel = path.strip_prefix(data_dir).unwrap_or(&path).to_path_buf();
        if rel.starts_with(&near_miss) {
            continue; // probes, not publications — CI excludes them too
        }
        if own_shards.contains(&rel) {
            continue;
        }
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
    if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
        for (i, line) in text.lines().enumerate() {
            collect_line(path, i + 1, line, ids)?;
        }
    } else {
        collect_line(path, 0, &text, ids)?;
    }
    Ok(())
}

/// Parse one record's text — a `.jsonl` line, or a whole `.json` file —
/// into the id set. `line` is 1-based, or 0 for a whole-file record.
fn collect_line(
    path: &Path,
    line: usize,
    text: &str,
    ids: &mut HashSet<String>,
) -> Result<(), String> {
    // Blank `.jsonl` lines are ordinary; an empty `.json` file is a
    // malformed published record — the corpus scan fails on it.
    if line != 0 && text.trim().is_empty() {
        return Ok(());
    }
    let v: Value = serde_json::from_str(text).map_err(|e| {
        let at = match line {
            0 => path.display().to_string(),
            n => format!("{}:{n}", path.display()),
        };
        format!("{at}: unparseable published record: {e}")
    })?;
    if let Some(id) = v["id"].as_str() {
        ids.insert(id.to_owned());
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
