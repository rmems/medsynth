//! Few-shot exemplars from the gold seed set (#7).
//!
//! Only the three flat `data/gold/<format>.jsonl` shards are read. The
//! `near-miss/` probes are deliberately defective and the review manifest is
//! not a record shard; neither is loaded here.

use std::collections::BTreeMap;
use std::path::Path;

use medsynth_schema::Domain;
use serde_json::Value;

use crate::error::GenerateError;
use crate::format::Format;
use crate::hash::{normalize_newlines, sha256_hex};
use crate::matrix::domain_slug;

/// Gold records keyed by format, plus a content hash so a batch manifest can
/// pin the exact exemplar inputs it ran against.
#[derive(Debug, Clone, Default)]
pub struct GoldCorpus {
    shards: BTreeMap<Format, Vec<Value>>,
    corpus_sha256: String,
}

impl GoldCorpus {
    /// Read the three gold shards from a directory.
    pub fn load(dir: &Path) -> Result<Self, GenerateError> {
        let mut shards = BTreeMap::new();
        let mut material = String::new();
        for format in Format::all() {
            let path = dir.join(format!("{}.jsonl", format.slug()));
            if !path.exists() {
                continue;
            }
            let text =
                std::fs::read_to_string(&path).map_err(|e| GenerateError::io(path.clone(), e))?;
            let records = parse_shard(&path, &text)?;
            material.push_str(format.slug());
            // Hash normalized text so CRLF checkouts don't change the corpus
            // hash (and therefore the batch id) for identical records.
            material.push_str(&sha256_hex(normalize_newlines(&text).as_bytes()));
            shards.insert(format, records);
        }
        Ok(Self {
            shards,
            corpus_sha256: sha256_hex(material.as_bytes()),
        })
    }

    /// SHA-256 over the corpus shards — the exemplar inputs a batch ran
    /// against. Empty corpus hashes as the empty input list.
    pub fn corpus_sha256(&self) -> &str {
        &self.corpus_sha256
    }

    /// The shard for a format.
    pub fn records(&self, format: Format) -> &[Value] {
        self.shards.get(&format).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Choose up to `limit` exemplars, preferring the same domain. Selection
    /// is id-sorted and deterministic.
    pub fn exemplars(&self, format: Format, domain: Domain, limit: usize) -> Vec<&Value> {
        if limit == 0 {
            return Vec::new();
        }
        let records = self.records(format);
        let slug = domain_slug(domain);
        let mut same_domain: Vec<&Value> = records
            .iter()
            .filter(|value| value.get("domain").and_then(Value::as_str) == Some(slug))
            .collect();
        if same_domain.is_empty() {
            same_domain = records.iter().collect();
        }
        same_domain.truncate(limit);
        same_domain
    }
}

/// Parse one shard: every non-blank line must be valid JSON and stay
/// `unverified`. Records come back id-sorted.
fn parse_shard(path: &Path, text: &str) -> Result<Vec<Value>, GenerateError> {
    let mut records = Vec::new();
    for (index, line) in text.lines().enumerate() {
        if let Some(value) = parse_gold_line(path, index, line)? {
            records.push(value);
        }
    }
    records.sort_by(|a, b| {
        a.get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .cmp(b.get("id").and_then(Value::as_str).unwrap_or_default())
    });
    Ok(records)
}

fn parse_gold_line(path: &Path, index: usize, line: &str) -> Result<Option<Value>, GenerateError> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    let value: Value = serde_json::from_str(line).map_err(|e| {
        GenerateError::Other(format!(
            "{}:{}: invalid gold JSON: {e}",
            path.display(),
            index + 1
        ))
    })?;
    if value.get("verification_status").and_then(Value::as_str) != Some("unverified") {
        return Err(GenerateError::Other(format!(
            "{}:{}: gold exemplar must stay unverified",
            path.display(),
            index + 1
        )));
    }
    Ok(Some(value))
}
