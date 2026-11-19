//! Record ids — unique slugs identifying a record across the dataset.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

/// Shape of a record id: a lowercase slug, 3–64 characters, no leading or
/// trailing hyphen.
pub const RECORD_ID_PATTERN: &str = r"^[a-z][a-z0-9-]{1,62}[a-z0-9]$";

/// A record's unique identifier within the dataset.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct RecordId(String);

impl RecordId {
    /// Whether `s` conforms to [`RECORD_ID_PATTERN`].
    pub fn is_valid(s: &str) -> bool {
        let b = s.as_bytes();
        if !(3..=64).contains(&b.len()) {
            return false;
        }
        if !b[0].is_ascii_lowercase()
            || !(b[b.len() - 1].is_ascii_lowercase() || b[b.len() - 1].is_ascii_digit())
        {
            return false;
        }
        b[1..b.len() - 1]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
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
