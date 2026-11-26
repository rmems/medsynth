//! The teacher pin registry (`docs/teachers.md`) as the admission gate needs
//! it: which pins exist, and the SHA-256 of each exact pin block. Generation
//! writes that block and hash into every teacher-lane rights envelope; stage 0
//! binds the envelope back to this trusted copy rather than trusting the
//! envelope's self-consistency alone.
//!
//! The block extraction and field syntax mirror `medsynth-generate`'s teacher
//! pin parser: `### Pin:` headings through the next heading, with fields in
//! `- **key:** \`value\`` form. Hashes cover the trimmed block with LF line
//! endings — the same canonical text generation hashes into `pin_entry_sha256`.

use std::collections::BTreeMap;
use std::path::Path;

use super::hash::{normalize_newlines, sha256_str};

/// One trusted pin entry, reduced to what the admission gate checks.
#[derive(Debug, Clone)]
pub struct TrustedTeacherPin {
    pub pin_id: String,
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
        let blocks = pin_blocks(&text);
        if blocks.is_empty() {
            return Err(format!(
                "{}: no `### Pin:` blocks in teacher pin registry",
                path.display()
            ));
        }
        let mut pins = BTreeMap::new();
        for entry_text in blocks {
            let pin = parse_pin(&entry_text);
            if pin.pin_id.trim().is_empty() {
                return Err(format!(
                    "{}: teacher pin registry has a Pin block with a blank or missing pin_id",
                    path.display()
                ));
            }
            if pins.insert(pin.pin_id.clone(), pin).is_some() {
                return Err(format!(
                    "{}: duplicate pin_id in teacher pin registry",
                    path.display()
                ));
            }
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

/// Extract each pin block exactly as generation's parser does: from the
/// `### Pin:` heading through the line before the next markdown heading.
fn pin_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut in_pin = false;
    for line in text.lines() {
        if line.starts_with("## ") || line.starts_with("### ") {
            if in_pin {
                blocks.push(join_block(&current));
                current.clear();
            }
            in_pin = line.starts_with("### Pin:") || line.starts_with("## Pin:");
        }
        if in_pin {
            current.push(line);
        }
    }
    if in_pin {
        blocks.push(join_block(&current));
    }
    blocks
}

fn join_block(lines: &[&str]) -> String {
    lines.join("\n").trim().to_owned()
}

/// Parse the one field stage 0 indexes by. Prose and the remaining fields stay
/// part of the hashed entry but are not interpreted by admission.
fn parse_pin(entry_text: &str) -> TrustedTeacherPin {
    let mut pin_id = String::new();
    for line in entry_text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("- **") else {
            continue;
        };
        let Some((key, value)) = rest.split_once(":**") else {
            continue;
        };
        if key.trim().eq_ignore_ascii_case("pin_id") {
            pin_id = clean_value(value);
        }
    }
    TrustedTeacherPin {
        pin_id,
        entry_sha256: sha256_str(&normalize_newlines(entry_text)),
    }
}

fn clean_value(value: &str) -> String {
    let value = value.trim();
    let value = value.strip_prefix('`').unwrap_or(value);
    let value = value.strip_suffix('`').unwrap_or(value);
    value.trim().to_owned()
}
