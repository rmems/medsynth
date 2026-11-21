//! The anchor source registry (`docs/sources.json`, issue #5).
//!
//! Generation may cite only registry ids (contract §4). The registry is read
//! as raw entry text so an anchor receipt can hash exactly the entry bytes the
//! draft was made against, rather than a reserialization that might drift.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::value::RawValue;

use crate::error::GenerateError;
use crate::hash::{file_sha256, sha256_hex};

/// Locator convention published for a source.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LocatorConvention {
    #[serde(default)]
    pub format: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub examples: Vec<String>,
}

/// One approved source, with the raw entry text and its hash.
#[derive(Debug, Clone)]
pub struct SourceEntry {
    pub source_id: String,
    pub kind: String,
    pub title: String,
    pub status: String,
    pub coverage: Vec<String>,
    pub locator_convention: LocatorConvention,
    /// Exact entry bytes from `docs/sources.json`.
    pub raw: String,
    /// SHA-256 of `raw`.
    pub sha256: String,
}

impl SourceEntry {
    pub fn is_current(&self) -> bool {
        self.status == "current"
    }

    pub fn covers(&self, subdomain_slug: &str) -> bool {
        self.coverage.iter().any(|c| c == subdomain_slug)
    }
}

/// The parsed registry.
#[derive(Debug, Clone)]
pub struct SourceRegistry {
    pub path: PathBuf,
    pub registry_version: String,
    pub file_sha256: String,
    entries: BTreeMap<String, SourceEntry>,
}

#[derive(Deserialize)]
struct RegistryFile {
    #[serde(default)]
    registry_version: String,
    #[serde(default)]
    sources: Vec<Box<RawValue>>,
}

#[derive(Deserialize)]
struct SourceEntryJson {
    source_id: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    coverage: Vec<String>,
    #[serde(default)]
    locator_convention: LocatorConvention,
}

impl SourceRegistry {
    /// Read and validate the registry.
    pub fn load(path: &Path) -> Result<Self, GenerateError> {
        let text =
            std::fs::read_to_string(path).map_err(|e| GenerateError::io(path.to_path_buf(), e))?;
        let parsed: RegistryFile = serde_json::from_str(&text)
            .map_err(|e| GenerateError::Registry(format!("{}: {e}", path.display())))?;
        if parsed.registry_version.trim().is_empty() {
            return Err(GenerateError::Registry(format!(
                "{}: registry_version is blank",
                path.display()
            )));
        }
        let mut entries = BTreeMap::new();
        for raw in parsed.sources {
            let raw_text = raw.get().to_owned();
            let entry: SourceEntryJson = serde_json::from_str(&raw_text).map_err(|e| {
                GenerateError::Registry(format!("{}: invalid source entry: {e}", path.display()))
            })?;
            if entry.source_id.trim().is_empty() {
                return Err(GenerateError::Registry(format!(
                    "{}: source entry has a blank source_id",
                    path.display()
                )));
            }
            let sha256 = sha256_hex(raw_text.as_bytes());
            let source = SourceEntry {
                source_id: entry.source_id.clone(),
                kind: entry.kind,
                title: entry.title,
                status: entry.status,
                coverage: entry.coverage,
                locator_convention: entry.locator_convention,
                raw: raw_text,
                sha256,
            };
            if entries.insert(entry.source_id.clone(), source).is_some() {
                return Err(GenerateError::Registry(format!(
                    "{}: duplicate source_id `{}`",
                    path.display(),
                    entry.source_id
                )));
            }
        }
        if entries.is_empty() {
            return Err(GenerateError::Registry(format!(
                "{}: registry has no sources",
                path.display()
            )));
        }
        let file_sha256 =
            file_sha256(path).map_err(|e| GenerateError::io(path.to_path_buf(), e))?;
        Ok(Self {
            path: path.to_path_buf(),
            registry_version: parsed.registry_version,
            file_sha256,
            entries,
        })
    }

    pub fn get(&self, source_id: &str) -> Option<&SourceEntry> {
        self.entries.get(source_id)
    }

    pub fn contains(&self, source_id: &str) -> bool {
        self.entries.contains_key(source_id)
    }

    /// Hash of the cited entry, used by anchor receipts.
    pub fn entry_sha256(&self, source_id: &str) -> Option<&str> {
        self.entries.get(source_id).map(|e| e.sha256.as_str())
    }

    /// Current registry entries covering a subdomain, in source-id order.
    pub fn current_covering(&self, subdomain_slug: &str) -> Vec<&SourceEntry> {
        self.entries
            .values()
            .filter(|e| e.is_current() && e.covers(subdomain_slug))
            .collect()
    }

    /// All entries, in source-id order.
    pub fn entries(&self) -> impl Iterator<Item = &SourceEntry> {
        self.entries.values()
    }
}
