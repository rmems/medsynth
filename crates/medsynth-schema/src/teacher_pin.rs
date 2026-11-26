//! Canonical parsing of the `docs/teachers.md` pin registry.
//!
//! Generation interprets the complete entries when preparing a teacher;
//! admission needs only their ids and exact hashes. Both crates use this one
//! extraction implementation so a registry block can never hash one way when
//! written into an envelope and another way when checked at intake.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

/// One extracted `### Pin:` block and its normalized fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeacherPinEntry {
    /// The entry's `pin_id` field, without surrounding backticks.
    pub pin_id: String,
    /// Exact canonical block text: trimmed, with LF line endings.
    pub entry_text: String,
    /// SHA-256 of `entry_text`.
    pub entry_sha256: String,
    fields: std::collections::BTreeMap<String, String>,
}

impl TeacherPinEntry {
    /// A normalized lowercase field value, if present.
    pub fn field(&self, key: impl AsRef<str>) -> Option<&str> {
        self.fields
            .get(&key.as_ref().to_ascii_lowercase())
            .map(String::as_str)
    }

    /// A pin-document boolean (`yes`/`true`/`1` or `no`/`false`/`0`).
    pub fn bool_field(&self, key: impl AsRef<str>) -> Option<bool> {
        match self.field(key)?.trim().to_ascii_lowercase().as_str() {
            "yes" | "true" | "1" => Some(true),
            "no" | "false" | "0" => Some(false),
            _ => None,
        }
    }
}

/// Errors common to every consumer of the pin registry.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TeacherPinIndexError {
    #[error("teacher pin registry has a Pin block with a blank or missing pin_id")]
    BlankPinId,
    #[error("duplicate pin_id `{0}` in teacher pin registry")]
    DuplicatePinId(String),
}

/// Extract every pin block in canonical form.
///
/// A block starts at a `### Pin:` or `## Pin:` heading and ends immediately
/// before the next level-two or level-three markdown heading. CRLF and lone CR
/// newlines are canonicalized before extraction, so the resulting entry text
/// is stable across platform checkouts.
pub fn parse_teacher_pin_entries(text: impl AsRef<str>) -> Vec<TeacherPinEntry> {
    let normalized = CanonicalPinText::new(text);
    let mut entries = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut in_pin = false;
    for line in normalized.0.lines() {
        if line.starts_with("## ") || line.starts_with("### ") {
            if in_pin {
                entries.push(parse_entry(join_block(&current)));
                current.clear();
            }
            in_pin = line.starts_with("### Pin:") || line.starts_with("## Pin:");
        }
        if in_pin {
            current.push(line);
        }
    }
    if in_pin {
        entries.push(parse_entry(join_block(&current)));
    }
    entries
}

/// Reject blank, missing, or duplicated pin ids before a consumer indexes the
/// entries. The exact registry path is added by the caller's error context.
pub fn ensure_unique_pin_ids(entries: &[TeacherPinEntry]) -> Result<(), TeacherPinIndexError> {
    let mut seen = BTreeSet::new();
    for entry in entries {
        if entry.pin_id.trim().is_empty() {
            return Err(TeacherPinIndexError::BlankPinId);
        }
        if !seen.insert(entry.pin_id.as_str()) {
            return Err(TeacherPinIndexError::DuplicatePinId(entry.pin_id.clone()));
        }
    }
    Ok(())
}

/// SHA-256 of a pin entry after the same newline canonicalization used at
/// extraction time. Tests and envelope builders use this instead of hashing
/// raw, platform-dependent text bytes.
pub fn teacher_pin_entry_sha256(entry_text: impl AsRef<str>) -> String {
    CanonicalPinText::new(entry_text).sha256()
}

/// Text whose line endings have one canonical representation. Keeping the
/// normalization behind this type prevents raw checkout text and canonical
/// entry text from being mixed up inside the parser.
struct CanonicalPinText(String);

impl CanonicalPinText {
    fn new(text: impl AsRef<str>) -> Self {
        Self(text.as_ref().replace("\r\n", "\n").replace('\r', "\n"))
    }

    fn sha256(&self) -> String {
        sha256_bytes(self.0.as_bytes())
    }

    fn into_string(self) -> String {
        self.0
    }
}

/// A raw markdown field value before its optional backtick quoting is removed.
struct RawPinValue<'a>(&'a str);

impl RawPinValue<'_> {
    fn clean(self) -> String {
        let value = self.0.trim();
        let value = value.strip_prefix('`').unwrap_or(value);
        let value = value.strip_suffix('`').unwrap_or(value);
        value.trim().to_owned()
    }
}

fn parse_entry(entry_text: CanonicalPinText) -> TeacherPinEntry {
    let entry_sha256 = entry_text.sha256();
    let entry_text = entry_text.into_string();
    let mut fields = std::collections::BTreeMap::new();
    for line in entry_text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("- **") else {
            continue;
        };
        let Some((key, value)) = rest.split_once(":**") else {
            continue;
        };
        fields.insert(key.trim().to_ascii_lowercase(), RawPinValue(value).clean());
    }
    let pin_id = fields.get("pin_id").cloned().unwrap_or_default();
    TeacherPinEntry {
        pin_id,
        entry_text,
        entry_sha256,
        fields,
    }
}

fn join_block(lines: &[&str]) -> CanonicalPinText {
    CanonicalPinText(lines.join("\n").trim().to_owned())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{parse_teacher_pin_entries, teacher_pin_entry_sha256};

    const PIN_BLOCK: &str = "### Pin: `example`\n\n- **pin_id:** `example`";
    const PIN_BLOCK_SHA256: &str =
        "716087ef98597b6b621a7377eb5f2c37921b21c57a81b1e397a526e9e7132139";

    #[test]
    fn pin_hash_is_fixed_across_newline_styles() {
        let crlf = PIN_BLOCK.replace('\n', "\r\n");
        let lone_cr = PIN_BLOCK.replace('\n', "\r");
        for text in [PIN_BLOCK, crlf.as_str(), lone_cr.as_str()] {
            let entries = parse_teacher_pin_entries(text);
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0].entry_sha256, PIN_BLOCK_SHA256);
            assert_eq!(
                teacher_pin_entry_sha256(&entries[0].entry_text),
                PIN_BLOCK_SHA256
            );
        }
    }
}
