//! Writing a batch to disk: per-format JSONL shards plus `manifest.json`.
//!
//! Drafts are staged to `<name>.tmp` and renamed over the previous file so an
//! interrupted write can never leave a half-written shard or manifest behind.
//! Shards for formats this batch attempted but did not produce are removed —
//! a stale shard from an earlier run must not masquerade as part of the new
//! batch. A leftover shard for a format the run never attempted (e.g. a
//! refused teacher lane) fails closed instead of being kept: writing a new
//! manifest over it would leave generated records without manifest coverage
//! or rights envelopes, so the run demands a fresh `--out` directory.
//!
//! The output directory is fail-closed against `data/`: drafts never enter
//! `data/gold/` or the admitted-record tree. The check resolves the output
//! path's deepest existing ancestor, so `..` traversal, symlinks and
//! not-yet-created leaves cannot bypass it.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use crate::batch::{BatchOutput, PlanPaths, protected_dirs};
use crate::error::GenerateError;
use crate::format::Format;
use crate::hash::display_path;

impl BatchOutput {
    /// Write one JSONL shard per produced format plus `manifest.json`.
    ///
    /// The output directory may not be under `data/`: admitted records belong
    /// in `data/<format>/` only after #9 admission, and drafts must never be
    /// written into `data/gold/`.
    pub fn write_output(&self, out_dir: &Path) -> Result<Vec<PathBuf>, GenerateError> {
        check_protected(out_dir, &self.protected_dirs)?;
        std::fs::create_dir_all(out_dir)
            .map_err(|e| GenerateError::io(out_dir.to_path_buf(), e))?;

        let mut grouped: BTreeMap<Format, Vec<&crate::batch::OutputRecord>> = BTreeMap::new();
        for record in &self.records {
            grouped
                .entry(record.record.format())
                .or_default()
                .push(record);
        }
        check_stale_shards(out_dir, &grouped, &self.attempted_formats)?;
        let mut written = Vec::new();
        for format in Format::all() {
            let path = out_dir.join(format!("{}.jsonl", format.slug()));
            match grouped.get(&format) {
                Some(records) => {
                    let mut text = String::new();
                    for record in records {
                        text.push_str(&record.raw);
                        text.push('\n');
                    }
                    atomic_write(&path, &text)?;
                    written.push(path);
                }
                None => {
                    // Attempted but produced nothing: drop the stale shard so
                    // it cannot masquerade as part of the new batch.
                    if path.exists() && self.attempted_formats.contains(&format) {
                        std::fs::remove_file(&path)
                            .map_err(|e| GenerateError::io(path.clone(), e))?;
                    }
                }
            }
        }
        let manifest_path = out_dir.join("manifest.json");
        atomic_write(&manifest_path, &self.manifest.to_json_string()?)?;
        written.push(manifest_path);
        Ok(written)
    }
}

/// Fail closed when `out_dir` already holds a shard for a format this run
/// neither produced nor attempted — the new manifest would cover only part of
/// the directory's generated content, leaving the stale shard's records
/// without rights envelopes.
fn check_stale_shards(
    out_dir: &Path,
    grouped: &BTreeMap<Format, Vec<&crate::batch::OutputRecord>>,
    attempted: &std::collections::BTreeSet<Format>,
) -> Result<(), GenerateError> {
    for format in Format::all() {
        if grouped.contains_key(&format) || attempted.contains(&format) {
            continue;
        }
        let path = out_dir.join(format!("{}.jsonl", format.slug()));
        if path.exists() {
            return Err(GenerateError::Config(format!(
                "{}: leftover `{}` shard from a previous run has no coverage in this batch's manifest — remove it or use a fresh --out directory",
                display_path(out_dir),
                format.slug()
            )));
        }
    }
    Ok(())
}

/// Fail closed if `out_dir` resolves into `data/` for these plan paths.
///
/// Public so the CLI can check `--out` before any work (in particular before
/// any teacher request burns API quota).
pub fn check_output_dir(paths: &PlanPaths, out_dir: &Path) -> Result<(), GenerateError> {
    check_protected(out_dir, &protected_dirs(paths))
}

fn check_protected(out_dir: &Path, protected: &[PathBuf]) -> Result<(), GenerateError> {
    let resolved = resolve_under_existing(out_dir);
    for dir in protected {
        // Canonicalize when the dir exists; otherwise resolve through its
        // deepest existing ancestor so a not-yet-created `data/` still
        // protects.
        let protected = dir
            .canonicalize()
            .unwrap_or_else(|_| resolve_under_existing(dir));
        if resolved == protected || resolved.starts_with(&protected) {
            return Err(GenerateError::Config(format!(
                "refusing to write drafts under `data/` (got `{}` resolving into `{}`); admitted records belong in data/<format>/ only after #9",
                display_path(out_dir),
                display_path(&resolved)
            )));
        }
    }
    Ok(())
}

/// Resolve `out_dir` through its deepest existing ancestor.
///
/// `create_dir_all` follows symlinks, so the check must too: the deepest
/// existing ancestor is canonicalized (resolving symlinks and `.`/`..`) and
/// the non-existing tail is re-appended. The result is where the write would
/// actually land.
fn resolve_under_existing(out_dir: &Path) -> PathBuf {
    let normalized = normalize_path(out_dir);
    let mut ancestor = normalized.clone();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while !ancestor.exists() {
        match (ancestor.file_name(), ancestor.parent()) {
            (Some(name), Some(parent)) => {
                tail.push(name.to_os_string());
                ancestor = parent.to_path_buf();
            }
            _ => break,
        }
    }
    let mut resolved = ancestor.canonicalize().unwrap_or(ancestor);
    for component in tail.iter().rev() {
        resolved.push(component);
    }
    resolved
}

/// Lexically collapse `.` and `..` so `drafts/../data/gold` cannot hide the
/// `data` prefix from the ancestor lookup.
fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // A leading `..` has no earlier component to cancel: keep it
                // so `../data` resolves outside the workspace instead of
                // being collapsed into the workspace's `data/`.
                if !normalized.pop() {
                    normalized.push(Component::ParentDir.as_os_str());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// Write via a sibling temp file and rename, so readers never see a
/// partially written shard or manifest. `std::fs::rename` replaces an
/// existing destination atomically on both Unix and Windows
/// (MOVEFILE_REPLACE_EXISTING); a separate delete step would reopen a
/// missing-file window.
fn atomic_write(path: &Path, text: &str) -> Result<(), GenerateError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text).map_err(|e| GenerateError::io(tmp.clone(), e))?;
    std::fs::rename(&tmp, path).map_err(|e| GenerateError::io(tmp.clone(), e))
}
