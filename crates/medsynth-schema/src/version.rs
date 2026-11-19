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
        // The raw JSON token preserves exactly what the record carries.
        // Comparing an f64 would silently round 1.0000000000000001 into 1;
        // comparing a Number would accept serde_json's arbitrary-precision
        // object marker ({"$serde_json::private::Number": "1"}) as the
        // version, which the schema's `const: 1` rejects as an object. This
        // ties record loading to serde_json's raw-token support — fine here,
        // since records are JSON documents by definition.
        let raw = <Box<serde_json::value::RawValue>>::deserialize(deserializer)?;
        let token = raw.get().trim();
        if is_exactly_current(token) {
            Ok(Self)
        } else {
            Err(D::Error::custom(format!(
                "unsupported schema_version {token}; this build writes and reads v{CURRENT_SCHEMA_VERSION}"
            )))
        }
    }
}

/// Whether a raw JSON token is exactly the current version under JSON Schema
/// numeric semantics: `1`, `1.0`, `1e0`, `10e-1`, `0.1e1` — any exact decimal
/// equal to it — but not `1.5`, `2`, `1.0000000000000001`, or any non-number.
///
/// The token's numeric value is `digits × 10^(exp - frac_len)` where `digits`
/// is the mantissa without its decimal point. For a single-digit version that
/// equals `CURRENT` iff `digits` is `CURRENT` followed by exactly
/// `frac_len - exp` zeros — compared as a string so no precision is lost.
/// Arithmetic is checked and the length is verified before any allocation:
/// an exponent like `i64::MIN` must fail cleanly, not overflow or allocate.
fn is_exactly_current(token: &str) -> bool {
    let (mantissa, exp) = match token.find(['e', 'E']) {
        Some(i) => (&token[..i], token[i + 1..].parse::<i64>().ok()),
        None => (token, Some(0)),
    };
    let Some(exp) = exp else { return false };
    if !mantissa.bytes().next().is_some_and(|b| b.is_ascii_digit()) {
        return false;
    }
    let (int_part, frac) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let Some(zeros_needed) = (frac.len() as i64).checked_sub(exp) else {
        return false;
    };
    if zeros_needed < 0 {
        return false;
    }
    let digits: String = format!("{int_part}{frac}");
    let digits = digits.trim_start_matches('0');
    // `digits` is `CURRENT` followed by exactly `zeros_needed` zeros iff its
    // length matches — checked before the repeat, so a huge negative
    // exponent exits here instead of allocating.
    if digits.len() as i64 - 1 != zeros_needed {
        return false;
    }
    let expected = format!(
        "{CURRENT_SCHEMA_VERSION}{}",
        "0".repeat(zeros_needed as usize)
    );
    digits == expected
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
