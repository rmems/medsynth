//! Anchored text blocks — record prose with the anchors that support it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anchor::Anchor;

/// A block of record text together with the source anchors supporting its
/// clinical and scientific claims (contract §4).
///
/// `anchors` may be empty only when the text asserts no clinical or
/// scientific fact — pure pedagogy (definitions, instructions to the
/// learner), or fictional-patient observations, which are invented and exempt
/// from anchoring. The JSON Schema cannot tell a claim from pedagogy; the
/// verification pipeline (#9) checks that every claim is anchored and that
/// each cited passage actually supports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnchoredText {
    /// The text itself.
    #[schemars(length(min = 1))]
    pub text: String,
    /// Source anchors for the claims this text makes. Empty only for pure
    /// pedagogy or invented fictional-patient observations (contract §4).
    pub anchors: Vec<Anchor>,
}

/// A block of record text that is definitionally a clinical or scientific
/// claim — a rationale, an expected answer, or a teaching point — so at least
/// one source anchor is required (contract §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnchoredClaim {
    /// The claim itself.
    #[schemars(length(min = 1))]
    pub text: String,
    /// Source anchors supporting the claim. At least one is required; the
    /// cited passage must actually support it (checked by #9, not here).
    #[schemars(length(min = 1))]
    pub anchors: Vec<Anchor>,
}
