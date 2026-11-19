//! The tutoring example record (format `tutoring_example`), the SFT format.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::anchor::Anchor;
use crate::error::ValidationError;
use crate::id::RecordId;
use crate::labels::RecordLabel;
use crate::provenance::{Provenance, VerificationStatus};
use crate::taxonomy::{Domain, Subdomain, Tier};
use crate::text::AnchoredText;
use crate::version::SchemaVersion;

/// A worked instructional example — one learner prompt and one tutor
/// response — for supervised fine-tuning (taxonomy §3.3 `tutoring_example`,
/// tier definitions §4.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutoringExampleRecord {
    /// Version of this record schema.
    pub schema_version: SchemaVersion,
    /// Unique record identifier.
    pub id: RecordId,
    /// Taxonomy domain (taxonomy §3.1).
    pub domain: Domain,
    /// Taxonomy subdomain; must belong to `domain` (taxonomy §3.2). Checked
    /// by [`TutoringExampleRecord::validate`].
    pub subdomain: Subdomain,
    /// Difficulty tier, interpreted per the `tutoring_example` definitions
    /// (taxonomy §4.3).
    pub tier: Tier,
    /// What the learner asks or is working through. The learner's own wording
    /// is not a medsynth claim — anchors attach only if the prompt itself
    /// asserts a clinical or scientific fact the record relies on.
    pub learner_prompt: AnchoredText,
    /// The tutor's reply, in Socratic or explanatory style per contract §5.
    pub tutor_response: TutorResponse,
    /// How the record was produced.
    pub provenance: Provenance,
    /// Verification status; advanced only by the pipeline in #9.
    pub verification_status: VerificationStatus,
    /// Record-level metadata labels; empty in the ordinary case. See
    /// [`RecordLabel`] for the closed set — e.g. `historical_superseded`
    /// marks a record retained on superseded sources (contract §7).
    #[serde(default)]
    pub labels: Vec<RecordLabel>,
    /// Version of `docs/taxonomy.md` this record was classified under
    /// (taxonomy §6).
    #[schemars(length(min = 1))]
    pub taxonomy_version: String,
    /// Version of `docs/contract.md` this record was admitted under (contract
    /// §9). `null` until the admission gate (#9) sets it; required non-blank
    /// once `verification_status` is `admitted` — enforced by
    /// [`TutoringExampleRecord::validate`].
    pub contract_version: Option<String>,
}

impl TutoringExampleRecord {
    /// Cross-field rules JSON Schema cannot express: the domain/subdomain
    /// pairing.
    ///
    /// Schema validation is still the first gate — run it before this.
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        crate::records::check_taxonomy(&mut errors, self.domain, self.subdomain);
        crate::records::check_contract_version(
            &mut errors,
            self.verification_status,
            self.contract_version.as_deref(),
        );
        crate::records::check_provenance(&mut errors, &self.provenance);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// The tutor's response to the learner — instructional framing ("consider,"
/// "the typical presentation is," "first-line management per [anchored
/// guideline]"), never directives to act on a real patient (contract §5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutorResponse {
    /// Whether the response teaches by questioning or by explaining.
    pub style: TutoringStyle,
    /// The response text.
    #[schemars(length(min = 1))]
    pub text: String,
    /// Source anchors for the claims the response makes. Empty only when the
    /// reply asserts no clinical or scientific fact — a Socratic response may
    /// teach by asking questions alone, which is claim-free pedagogy
    /// (contract §4). #9 checks that every claim made is anchored.
    pub anchors: Vec<Anchor>,
}

/// How the tutor response teaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TutoringStyle {
    /// Leads the learner with questions.
    Socratic,
    /// Explains directly, with explicit reasoning.
    Explanatory,
}

string_enum! {
    TutoringStyle,
    "socratic" => TutoringStyle::Socratic,
    "explanatory" => TutoringStyle::Explanatory,
}
