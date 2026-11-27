//! The teacher pin registry (`docs/teachers.md`) as the admission gate needs
//! it: which pins exist, and the SHA-256 of each exact pin block. Generation
//! writes that block and hash into every teacher-lane rights envelope; stage 0
//! binds the envelope back to this trusted copy rather than trusting the
//! envelope's self-consistency alone.
//!
//! Both lanes extract entries through `medsynth-schema`'s shared parser, so
//! the trusted hash admission checks is byte-for-byte the hash generation
//! writes into `pin_entry_sha256`.

use std::collections::BTreeMap;
use std::path::Path;

use medsynth_schema::teacher_pin::{ensure_unique_pin_ids, parse_teacher_pin_entries};

use super::hash::{normalize_newlines, sha256_str};

/// One trusted pin entry, reduced to what the admission gate checks.
#[derive(Debug, Clone)]
pub struct TrustedTeacherPin {
    pub entry_sha256: String,
}

/// The parsed trusted teacher-pin registry.
#[derive(Debug, Clone)]
pub struct TeacherPinRegistry {
    pub file_sha256: String,
    pins: BTreeMap<String, TrustedTeacherPin>,
}

impl TeacherPinRegistry {
    /// Read and parse the registry. An unreadable or malformed registry is an
    /// operational error: without the trusted pin list there is no rights
    /// envelope check to run, so the gate fails closed.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read teacher pin registry {}: {e}", path.display()))?;
        let entries = parse_teacher_pin_entries(&text);
        if entries.is_empty() {
            return Err(format!(
                "{}: no `### Pin:` blocks in teacher pin registry",
                path.display()
            ));
        }
        ensure_unique_pin_ids(&entries).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut pins = BTreeMap::new();
        for entry in entries {
            pins.insert(
                entry.pin_id.clone(),
                TrustedTeacherPin {
                    entry_sha256: entry.entry_sha256,
                },
            );
        }
        Ok(Self {
            file_sha256: sha256_str(&normalize_newlines(&text)),
            pins,
        })
    }

    pub fn get(&self, pin_id: &str) -> Option<&TrustedTeacherPin> {
        self.pins.get(pin_id)
    }
}
