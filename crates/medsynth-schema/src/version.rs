//! Schema versioning — every record carries the schema version it was written
//! against, and each generated schema file is versioned in its filename and
//! `$id` (`schemas/<format>.v<N>.json`).

use std::borrow::Cow;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};

/// The schema version this crate emits and accepts.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// The `schema_version` field on every record. Currently always `1`; a record
/// carrying any other version fails to deserialize under this build, so mixed
/// versions surface immediately rather than silently validating against the
/// wrong shape.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct SchemaVersion;

impl SchemaVersion {
    pub fn get(self) -> u32 {
        CURRENT_SCHEMA_VERSION
    }
}

impl Serialize for SchemaVersion {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(CURRENT_SCHEMA_VERSION)
    }
}

impl<'de> Deserialize<'de> for SchemaVersion {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // JSON Schema's numeric semantics treat 1.0 and 1e0 as the integer 1,
        // so the generated `const` accepts them; deserialization must match —
        // and must not round an unsupported version like 1.0000000000000001
        // into 1. serde_json's arbitrary_precision keeps the raw token, so
        // this compares the exact decimal the record carries.
        let v = serde_json::Number::deserialize(deserializer)?;
        if is_exactly_current(&v.to_string()) {
            Ok(Self)
        } else {
            Err(D::Error::custom(format!(
                "unsupported schema_version {v}; this build writes and reads v{CURRENT_SCHEMA_VERSION}"
            )))
        }
    }
}

/// Whether a raw JSON number token is exactly the current version: `1`, a
/// decimal `1.0…`, or either with a zero exponent (`1e0`).
fn is_exactly_current(token: &str) -> bool {
    let (mantissa, exponent) = match token.find(['e', 'E']) {
        Some(i) => (&token[..i], &token[i + 1..]),
        None => (token, "0"),
    };
    if !matches!(exponent.parse::<i64>(), Ok(0)) {
        return false;
    }
    let expected = CURRENT_SCHEMA_VERSION.to_string();
    let mut parts = mantissa.splitn(2, '.');
    if parts.next() != Some(expected.as_str()) {
        return false;
    }
    parts
        .next()
        .is_none_or(|frac| !frac.is_empty() && frac.bytes().all(|b| b == b'0'))
}

impl JsonSchema for SchemaVersion {
    fn schema_name() -> Cow<'static, str> {
        "SchemaVersion".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "integer",
            "const": CURRENT_SCHEMA_VERSION,
            "description": "Version of the record schema this record conforms to."
        })
    }
}
