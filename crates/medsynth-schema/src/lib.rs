//! Canonical Rust record types for medsynth's three record formats: the MCQ,
//! the case vignette, and the tutoring example.
//!
//! These types are the **source of truth**. The JSON Schemas in `schemas/`
//! are generated from them with schemars and checked in — regenerate them
//! with `cargo run -p medsynth-schema --bin schema-gen` after changing the
//! types; never hand-edit the generated files.
//!
//! Two layers of validation exist on purpose:
//!
//! * **JSON Schema** (`schemas/`) — the structural gate CI (#10) runs on
//!   every record with the `jsonschema` crate.
//! * **`validate()`** on each record type — cross-field rules JSON Schema
//!   cannot express, such as the domain/subdomain pairing (taxonomy §3.2)
//!   and the MCQ key matching an option label.
//!
//! The binding documents are `docs/contract.md` (accuracy & safety rules) and
//! `docs/taxonomy.md` (enum vocabulary and coverage); field docs cite their
//! sections.

/// Generates a strict string-only `Deserialize` for a string-valued enum.
///
/// serde's derived enum impl also accepts the externally-tagged object form —
/// `{"Variant": null}` loads as the unit variant — which the generated
/// schema's string `enum` rejects. Deserializing from a string keeps the two
/// layers in agreement.
macro_rules! string_enum {
    ($ty:ty, $($lit:literal => $variant:expr),+ $(,)?) => {
        impl<'de> ::serde::Deserialize<'de> for $ty {
            fn deserialize<D: ::serde::Deserializer<'de>>(
                deserializer: D,
            ) -> ::std::result::Result<Self, D::Error> {
                let s =
                    <::std::string::String as ::serde::Deserialize>::deserialize(deserializer)?;
                match s.as_str() {
                    $($lit => ::std::result::Result::Ok($variant),)+
                    _ => ::std::result::Result::Err(
                        <D::Error as ::serde::de::Error>::unknown_variant(&s, &[$($lit),+]),
                    ),
                }
            }
        }
    };
}

/// Version of `docs/contract.md` the workspace writes into records —
/// generators stamp it and the admission gate (#9) verifies against it
/// (contract §9).
pub const CONTRACT_VERSION: &str = "0.1";

pub mod anchor;
mod error;
mod id;
mod labels;
pub mod locator;
pub mod provenance;
mod records;
mod schemas;
pub mod taxonomy;
pub mod text;
mod version;

pub use anchor::{Anchor, InvalidSourceId, SOURCE_ID_PATTERN, SourceId};
pub use error::ValidationError;
pub use id::{InvalidRecordId, RECORD_ID_PATTERN, RecordId};
pub use labels::RecordLabel;
pub use provenance::{Provenance, VerificationStatus};
pub use records::{
    CaseVignetteRecord, McqOption, McqRationale, McqRecord, OptionLabel, TutorResponse,
    TutoringExampleRecord, TutoringStyle, VignetteQuestion,
};
pub use schemas::{SchemaFile, schema_files};
pub use taxonomy::{Domain, Subdomain, Tier};
pub use text::{AnchoredClaim, AnchoredText};
pub use version::{CURRENT_SCHEMA_VERSION, SchemaVersion};
