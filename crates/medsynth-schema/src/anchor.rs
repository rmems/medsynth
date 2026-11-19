//! Source anchors — the first-class citation attached to every clinical or
//! scientific fact in a record (contract §4).

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

/// Shape of a registry source id: a lowercase slug, 3–64 characters, no
/// leading or trailing hyphen.
///
/// The registry built by issue #5 assigns the actual ids; this type fixes
/// only the pattern they must follow. Ids are expected to be edition-scoped
/// (e.g. `some-textbook-7e`) so the cited edition or year is visible at a
/// glance per the contract's currency rule (§7). Which ids exist — and what
/// each covers — is the registry's concern, not this crate's.
pub const SOURCE_ID_PATTERN: &str = r"^[a-z][a-z0-9-]{1,62}[a-z0-9]$";

/// The id of a source in the anchor source registry (#5).
///
/// A `SourceId` guarantees the registry's slug format; it does not guarantee
/// the source exists — resolving the id against the registry is the
/// verification pipeline's job (#9).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct SourceId(String);

impl SourceId {
    /// Whether `s` conforms to [`SOURCE_ID_PATTERN`].
    pub fn is_valid(s: &str) -> bool {
        let b = s.as_bytes();
        if !(3..=64).contains(&b.len()) {
            return false;
        }
        if !b[0].is_ascii_lowercase() || !is_slug_end(b[b.len() - 1]) {
            return false;
        }
        b[1..b.len() - 1]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
    }

    /// Wrap a string, checking [`SOURCE_ID_PATTERN`].
    pub fn new(s: impl Into<String>) -> Result<Self, InvalidSourceId> {
        let s = s.into();
        if Self::is_valid(&s) {
            Ok(Self(s))
        } else {
            Err(InvalidSourceId(s))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn is_slug_end(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

/// Error returned when a source id does not match [`SOURCE_ID_PATTERN`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid source id `{0}`: must match {SOURCE_ID_PATTERN}")]
pub struct InvalidSourceId(String);

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SourceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(D::Error::custom)
    }
}

impl JsonSchema for SourceId {
    fn schema_name() -> Cow<'static, str> {
        "SourceId".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": SOURCE_ID_PATTERN,
            "description": "Id of a source in the anchor source registry: a lowercase slug of 3-64 characters. Registry ids are edition-scoped so the cited edition or year is visible (contract §7)."
        })
    }
}

/// A source anchor citing one registry source plus a locator inside it.
///
/// Anchors are first-class, structured fields — never free text. Per contract
/// §4, every clinical or scientific fact in a record carries one, and the
/// cited passage must actually support the claim it is attached to (checked
/// by the verification pipeline, #9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    /// Id of a source in the anchor source registry (#5). Only registry ids
    /// may be cited; a fact whose only support is an unlisted source is
    /// unanchored until that source is approved (contract §4).
    pub source_id: SourceId,
    /// Locator inside the source, written in the locator convention the
    /// registry publishes for that source (chapter + section, or page where
    /// the convention provides one — e.g. `ch34:cardiac-cycle` or `p.482`).
    /// The string is intentionally untyped until #5 publishes per-source
    /// locator conventions; this field exists so those conventions have
    /// somewhere to land.
    #[schemars(length(min = 1))]
    pub locator: String,
}
