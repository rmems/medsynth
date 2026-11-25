//! The pipeline's writes: admitted shards under `data/<format>/` and the
//! admission log — both staged through temp files so a failed run never
//! leaves a truncated artifact behind.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::Format;

use super::batch::DraftRecord;
use super::config::RunConfig;

/// Write admitted records to `data/<format>/<batch_id>.jsonl`, one compact
/// JSON line per record sorted by id — the only writes `data/` accepts.
/// A re-run merges with the existing shard rather than overwriting it:
/// admission is one-way, so records admitted by an earlier run of the
/// same batch stay admitted even when this run holds them.
pub(super) fn write_admitted(
    data_dir: &Path,
    batch_id: &str,
    admitted: &[&DraftRecord],
    config: &RunConfig,
) -> Result<Vec<String>, String> {
    let mut shards = Vec::new();
    for format in Format::all() {
        if admitted.iter().all(|r| r.format != format) {
            continue; // nothing new — the existing shard stands as-is
        }
        let path = data_dir
            .join(format.slug())
            .join(format!("{batch_id}.jsonl"));
        let mut records = existing_shard(&path)?;
        for record in admitted.iter().filter(|r| r.format == format) {
            let mut value = record.value.clone();
            value["verification_status"] = Value::String("admitted".to_owned());
            value["contract_version"] = Value::String(config.contract_version.clone());
            records.insert(record.record_id(), value);
        }
        std::fs::create_dir_all(path.parent().expect("shard paths have a parent"))
            .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        write_text(&path, &shard_text(&records)?)
            .map_err(|e| format!("cannot write shard {}: {e}", path.display()))?;
        shards.push(display(&path));
    }
    Ok(shards)
}

/// The records a shard already holds, keyed by id — kept verbatim so a
/// held re-run never un-admits them.
fn existing_shard(path: &Path) -> Result<BTreeMap<String, Value>, String> {
    let mut records = BTreeMap::new();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(records), // no shard yet
        // An unreadable shard must never read as an empty one — the merge
        // would overwrite it and un-admit every record it held.
        Err(e) => {
            return Err(format!(
                "cannot read existing shard {}: {e}",
                path.display()
            ));
        }
    };
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Value = serde_json::from_str(line).map_err(|e| {
            format!(
                "{}:{}: unparseable existing shard line: {e}",
                path.display(),
                i + 1
            )
        })?;
        if let Some(id) = v["id"].as_str() {
            records.insert(id.to_owned(), v);
        }
    }
    Ok(records)
}

/// The shard's contents: one compact JSON line per record, sorted by id.
fn shard_text(records: &BTreeMap<String, Value>) -> Result<String, String> {
    let mut text = String::new();
    for (id, value) in records {
        let line = serde_json::to_string(value).map_err(|e| format!("serialize {id}: {e}"))?;
        text.push_str(&line);
        text.push('\n');
    }
    Ok(text)
}

pub(super) fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    write_text(path, &text)
}

/// Write via a per-process temp file + rename so a failed run never leaves
/// a truncated shard or log behind. `create_new` on the staging path so a
/// planted link fails loudly instead of being followed.
fn write_text(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", super::fulfill::tmp_tag()));
    let result = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut f| io::Write::write_all(&mut f, text.as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Forward-slash path rendering for the log — Windows separators are a
/// platform detail, not report content.
pub(super) fn display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
