//! Record ids — unique slugs identifying a record across the dataset.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

/// Shape of a record id: a lowercase slug, 3–64 characters, no leading or
/// trailing hyphen. Ends with `(?![\s\S])` — a true end-of-input assertion —
/// because `$` in ECMA-style regexes can match before a final newline. The
/// lookahead needs an engine that supports it (ECMA, or fancy-regex as used
/// by the `jsonschema` crate); [`RecordId::is_valid`] is the portable check.
pub const RECORD_ID_PATTERN: &str = r"^[a-z][a-z0-9-]{1,62}[a-z0-9](?![\s\S])";

/// A record's unique identifier within the dataset.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct RecordId(String);

/// Whether `s` is a lowercase slug of 3–64 characters with no leading or
/// trailing hyphen — the shared shape behind [`RecordId::is_valid`] and
/// `SourceId::is_valid`.
pub(crate) fn is_slug(s: &str) -> bool {
    let b = s.as_bytes();
    (3..=64).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && is_slug_end(b[b.len() - 1])
        && b[1..b.len() - 1]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

pub(crate) fn is_slug_end(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

impl RecordId {
    /// Whether `s` conforms to [`RECORD_ID_PATTERN`].
    pub fn is_valid(s: &str) -> bool {
        is_slug(s)
    }

    /// Wrap a string, checking [`RECORD_ID_PATTERN`].
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidRecordId> {
        let s = s.into();
        if Self::is_valid(&s) {
            Ok(Self(s))
        } else {
            Err(InvalidRecordId(s))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Error returned when a record id does not match [`RECORD_ID_PATTERN`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid record id `{0}`: must match {RECORD_ID_PATTERN}")]
pub struct InvalidRecordId(String);

impl fmt::Display for RecordId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for RecordId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(D::Error::custom)
    }
}

impl JsonSchema for RecordId {
    fn schema_name() -> Cow<'static, str> {
        "RecordId".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": RECORD_ID_PATTERN,
            "description": "Unique record identifier: a lowercase slug of 3-64 characters."
        })
    }
}
