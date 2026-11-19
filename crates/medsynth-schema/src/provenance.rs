//! Provenance and verification status.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How a record was produced — enough to reproduce or audit its drafting.
///
/// Model-drafted records carry `model` and `prompt_version`;
/// template-generated records carry `template_version`. Hand-written records
/// (fixtures, the gold seed set) name their author or checklist in
/// `generator`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// Identifier of the generator that produced the record — a procedural
    /// template name, a teacher-model pipeline id, or a human author tag.
    #[schemars(length(min = 1))]
    pub generator: String,
    /// Teacher model identifier, when the record was model-drafted.
    pub model: Option<String>,
    /// Version of the prompt used with the teacher model, when applicable.
    pub prompt_version: Option<String>,
    /// Version of the procedural template, when template-generated.
    pub template_version: Option<String>,
}

/// Where a record sits on the path to the dataset (contract §8).
///
/// Exactly these four values. Only the verification & admission pipeline (#9)
/// may advance a record's status; generators always write `unverified`, and
/// nothing outside #9 writes `key-checked`, `anchor-checked`, or `admitted`.
/// This schema defines the enum — it does not perform the advancement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum VerificationStatus {
    /// Drafted but not yet checked; never reaches the dataset.
    Unverified,
    /// Answer key re-derived and confirmed (#9).
    KeyChecked,
    /// Source anchors resolved and confirmed to support their claims (#9).
    AnchorChecked,
    /// Passed verification and admitted into the dataset by the admission
    /// gate (#9); counts toward coverage-matrix fill (taxonomy §5).
    Admitted,
}

string_enum! {
    VerificationStatus,
    "unverified" => VerificationStatus::Unverified,
    "key-checked" => VerificationStatus::KeyChecked,
    "anchor-checked" => VerificationStatus::AnchorChecked,
    "admitted" => VerificationStatus::Admitted,
}
