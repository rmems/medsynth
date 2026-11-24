//! Loading a draft batch: the manifest sidecar plus the record files.
//!
//! A batch directory is what `medsynth-generate` writes — a `manifest.json`
//! plus one `<format>.jsonl` shard per format — or, for tests and staged
//! sets like the gold seeds, a plain directory of records (`<format>.jsonl`
//! shards or `<format>/*.json` files) with no manifest at all.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::Format;

use super::hash::{normalize_newlines, sha256_str};

/// One parsed record in the batch.
#[derive(Debug)]
pub struct DraftRecord {
    /// Record file path relative to the batch directory, for reports.
    pub path: PathBuf,
    /// 1-based line inside a `.jsonl` shard, or `None` for `.json` files.
    pub line: Option<usize>,
    /// The record format, from its directory or shard name.
    pub format: Format,
    /// The parsed record.
    pub value: Value,
    /// The exact record text — one compact JSON line for `.jsonl` records,
    /// the whole file for `.json` records. Content hashes cover this.
    pub raw: String,
}

impl DraftRecord {
    /// The record's `id` field, or a placeholder when unreadable.
    pub fn record_id(&self) -> String {
        self.value
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("<no-id>")
            .to_owned()
    }

    /// SHA-256 of the raw record text — the digest `rights_envelopes[].content_sha256`
    /// and `records[].sha256` pin in a #8 manifest.
    pub fn content_sha256(&self) -> String {
        sha256_str(&normalize_newlines(&self.raw))
    }
}

/// A loaded batch: the records plus the manifest's verification-relevant
/// views (record entries, rights envelopes, anchor receipts).
#[derive(Debug)]
pub struct Batch {
    /// The batch id: `manifest.batch_id` when present, else derived from the
    /// sorted record ids so re-runs of the same drafts stay identifiable.
    pub batch_id: String,
    pub records: Vec<DraftRecord>,
    manifest: Option<Value>,
}

impl Batch {
    /// Load a batch directory. Malformed or unclassifiable record files are
    /// hard errors — an admission gate must never skip a file silently.
    pub fn load(dir: &Path) -> Result<Self, String> {
        let mut files = Vec::new();
        walk(dir, dir, &mut files).map_err(|e| format!("cannot scan {}: {e}", dir.display()))?;
        files.sort();

        let manifest_path = dir.join("manifest.json");
        let manifest = match fs::read_to_string(&manifest_path) {
            Ok(text) => Some(
                serde_json::from_str::<Value>(&text)
                    .map_err(|e| format!("{}: {e}", manifest_path.display()))?,
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("cannot read {}: {e}", manifest_path.display())),
        };

        let mut records = Vec::new();
        for file in files {
            let rel = file.strip_prefix(dir).unwrap_or(&file).to_path_buf();
            records.extend(load_record_file(&file, &rel)?);
        }

        let batch_id = manifest
            .as_ref()
            .and_then(|m| m["batch_id"].as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                let mut ids: Vec<&str> = records
                    .iter()
                    .filter_map(|r| r.value["id"].as_str())
                    .collect();
                ids.sort_unstable();
                format!("batch-{}", &sha256_str(&ids.join("\n"))[..16])
            });
        Ok(Self {
            batch_id,
            records,
            manifest,
        })
    }

    pub fn has_manifest(&self) -> bool {
        self.manifest.is_some()
    }

    /// The `records[]` manifest entry for a record id, when the batch has
    /// a manifest that lists it.
    pub fn record_entry(&self, record_id: &str) -> Option<&Value> {
        self.manifest.as_ref()?["records"]
            .as_array()?
            .iter()
            .find(|e| e["id"].as_str() == Some(record_id))
    }

    /// The `rights_envelopes[]` entry for a record id, when present.
    pub fn rights_envelope(&self, record_id: &str) -> Option<&Value> {
        self.manifest.as_ref()?["rights_envelopes"]
            .as_array()?
            .iter()
            .find(|e| e["record_id"].as_str() == Some(record_id))
    }

    /// Whether the manifest carries an `anchor_receipts` section at all —
    /// receipt checks apply only when it does.
    pub fn has_anchor_receipts(&self) -> bool {
        self.manifest
            .as_ref()
            .is_some_and(|m| m["anchor_receipts"].as_array().is_some())
    }

    /// The `anchor_receipts[]` entries matching one record + anchor.
    pub fn anchor_receipt<'a>(
        &'a self,
        record_id: &str,
        source_id: &str,
        locator: &str,
    ) -> Option<&'a Value> {
        self.manifest.as_ref()?["anchor_receipts"]
            .as_array()?
            .iter()
            .find(|e| {
                e["record_id"].as_str() == Some(record_id)
                    && e["source_id"].as_str() == Some(source_id)
                    && e["locator"].as_str() == Some(locator)
            })
    }
}

/// Parse one record file: every line of a `.jsonl` shard, or the whole
/// file for a `.json` record.
fn load_record_file(file: &Path, rel: &Path) -> Result<Vec<DraftRecord>, String> {
    let Some(format) = format_of(rel) else {
        return Err(format!(
            "{}: record file lives outside a format directory \
             (mcq/ case_vignette/ tutoring_example/) and is not a \
             <format>.jsonl shard",
            rel.display()
        ));
    };
    let text =
        fs::read_to_string(file).map_err(|e| format!("cannot read {}: {e}", rel.display()))?;
    if !rel.extension().is_some_and(|e| e == "jsonl") {
        let value = serde_json::from_str(&text)
            .map_err(|e| format!("{}: invalid JSON: {e}", rel.display()))?;
        return Ok(vec![DraftRecord {
            path: rel.to_path_buf(),
            line: None,
            format,
            value,
            raw: text,
        }]);
    }
    text.lines()
        .enumerate()
        .map(|(idx, raw)| {
            if raw.trim().is_empty() {
                return Err(format!(
                    "{}:{}: blank line in JSONL shard",
                    rel.display(),
                    idx + 1
                ));
            }
            let value = serde_json::from_str(raw)
                .map_err(|e| format!("{}:{}: invalid JSON: {e}", rel.display(), idx + 1))?;
            Ok(DraftRecord {
                path: rel.to_path_buf(),
                line: Some(idx + 1),
                format,
                value,
                raw: raw.to_owned(),
            })
        })
        .collect()
}

/// Whether a batch-relative path classifies a record file: the first
/// component that is a format slug, or the file stem for a top-level
/// `<format>.jsonl` shard.
fn format_of(rel: &Path) -> Option<Format> {
    for component in rel.components() {
        if let Some(format) = Format::all()
            .into_iter()
            .find(|f| Some(f.slug()) == component.as_os_str().to_str())
        {
            return Some(format);
        }
    }
    rel.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|stem| Format::all().into_iter().find(|f| f.slug() == stem))
}

/// Files that are never records even though they end in `.json`.
fn is_sidecar(rel: &Path) -> bool {
    let Some(name) = rel.file_name().and_then(|n| n.to_str()) else {
        return true;
    };
    name == "manifest.json"
        || name.ends_with("-manifest.json")
        || name == "config.json"
        || name.starts_with("admission-log")
        || name.ends_with(".request.json")
        || name.ends_with(".response.json")
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_symlink() {
            continue;
        }
        if entry.file_type()?.is_dir() {
            walk(root, &path, out)?;
        } else {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let is_record = matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("json") | Some("jsonl")
            ) && !is_sidecar(rel);
            if is_record {
                out.push(path);
            }
        }
    }
    Ok(())
}
