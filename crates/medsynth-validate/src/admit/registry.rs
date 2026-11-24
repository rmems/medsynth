//! The anchor source registry (`docs/sources.json`) as the admission gate
//! needs it: entry status, locator convention, and the exact-entry SHA-256
//! that #8's anchor receipts pin, so a receipt can be checked against the
//! *current* registry rather than trusted (issue #9 stage 3).
//!
//! Entry hashes cover the entry's exact bytes inside `docs/sources.json`
//! after LF-normalization — the same bytes `medsynth-generate` hashed when
//! it wrote the receipt, so a registry edit since drafting is detectable.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::value::RawValue;

use super::hash::{normalize_newlines, sha256_str};

/// One registry source, with its status and locator convention.
#[derive(Debug, Clone)]
pub struct SourceEntry {
    pub status: String,
    /// The `locator_convention.format` string — mapped to a regex by
    /// `medsynth_schema::locator::pattern`.
    pub locator_format: String,
    /// SHA-256 of the entry's exact bytes in the registry file.
    pub sha256: String,
}

impl SourceEntry {
    pub fn is_current(&self) -> bool {
        self.status == "current"
    }
}

/// The parsed registry.
#[derive(Debug, Clone)]
pub struct SourceRegistry {
    pub registry_version: String,
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
    status: String,
    #[serde(default)]
    locator_convention: LocatorConvention,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct LocatorConvention {
    #[serde(default)]
    format: String,
}

impl SourceRegistry {
    /// Read the registry, hashing each entry's exact bytes for receipt
    /// checks. An unreadable or malformed registry is an operational error —
    /// the gate fails closed rather than treating every anchor as unlisted.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read registry {}: {e}", path.display()))?;
        let parsed: RegistryFile =
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if parsed.registry_version.trim().is_empty() {
            return Err(format!("{}: registry_version is blank", path.display()));
        }
        let mut entries = BTreeMap::new();
        for raw in parsed.sources {
            let raw_text = normalize_newlines(raw.get());
            let entry: SourceEntryJson = serde_json::from_str(&raw_text)
                .map_err(|e| format!("{}: invalid source entry: {e}", path.display()))?;
            if entry.source_id.trim().is_empty() {
                return Err(format!(
                    "{}: source entry has a blank source_id",
                    path.display()
                ));
            }
            let sha256 = sha256_str(&raw_text);
            if entries
                .insert(
                    entry.source_id.clone(),
                    SourceEntry {
                        status: entry.status,
                        locator_format: entry.locator_convention.format,
                        sha256,
                    },
                )
                .is_some()
            {
                return Err(format!(
                    "{}: duplicate source_id `{}`",
                    path.display(),
                    entry.source_id
                ));
            }
        }
        Ok(Self {
            registry_version: parsed.registry_version,
            entries,
        })
    }

    pub fn get(&self, source_id: &str) -> Option<&SourceEntry> {
        self.entries.get(source_id)
    }
}
