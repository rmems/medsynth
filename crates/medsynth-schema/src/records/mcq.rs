//! The multiple-choice question record (format `mcq`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::ValidationError;
use crate::id::RecordId;
use crate::labels::RecordLabel;
use crate::provenance::{Provenance, VerificationStatus};
use crate::taxonomy::{Domain, Subdomain, Tier};
use crate::text::{AnchoredClaim, AnchoredText};
use crate::version::SchemaVersion;

/// A single-best-answer multiple-choice question (taxonomy §3.3 `mcq`,
/// tier definitions §4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McqRecord {
    /// Version of this record schema.
    pub schema_version: SchemaVersion,
    /// Unique record identifier.
    pub id: RecordId,
    /// Taxonomy domain (taxonomy §3.1).
    pub domain: Domain,
    /// Taxonomy subdomain; must belong to `domain` (taxonomy §3.2). Checked
    /// by [`McqRecord::validate`] — JSON Schema cannot express the pairing.
    pub subdomain: Subdomain,
    /// Difficulty tier, interpreted per the `mcq` definitions (taxonomy §4.1).
    pub tier: Tier,
    /// The question stem. Factual claims in the stem carry anchors on this
    /// field (contract §4).
    pub stem: AnchoredText,
    /// Answer options: 4–5, each with a unique `A`–`E` label.
    #[schemars(length(min = 4, max = 5))]
    pub options: Vec<McqOption>,
    /// Label of the correct option. Must match one of `options[].label` —
    /// enforced by [`McqRecord::validate`], beyond what JSON Schema expresses.
    #[schemars(regex(pattern = "^[A-E]$"))]
    pub key: String,
    /// Why the key is the best answer and why the strongest distractor is
    /// wrong. Distractors are anchored through the refutation here, not on
    /// the option itself (contract §4).
    pub rationale: McqRationale,
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
    /// [`McqRecord::validate`].
    pub contract_version: Option<String>,
}

impl McqRecord {
    /// Cross-field rules JSON Schema cannot express: the domain/subdomain
    /// pairing, unique option labels, and the key matching a real option.
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
        let mut seen = std::collections::HashSet::new();
        for option in &self.options {
            if !seen.insert(option.label.clone()) {
                errors.push(ValidationError::DuplicateOptionLabel {
                    label: option.label.clone(),
                });
            }
        }
        if !self.options.iter().any(|o| o.label == self.key) {
            errors.push(ValidationError::KeyMatchesNoOption {
                key: self.key.clone(),
            });
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// One answer option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McqOption {
    /// Option label, `A`–`E`, unique within the record.
    #[schemars(regex(pattern = "^[A-E]$"))]
    pub label: String,
    /// Option text. Carries no anchors: a distractor is a wrong answer by
    /// design and is anchored through the rationale's refutation instead
    /// (contract §4).
    #[schemars(length(min = 1))]
    pub text: String,
}

/// The MCQ rationale — the issue requires explaining the key *and* why the
/// strongest distractor is wrong, so the two halves are separate fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct McqRationale {
    /// Why the keyed option is the best answer (mechanism, guideline
    /// concordance, or classic teaching point — contract §5).
    pub key_explanation: AnchoredClaim,
    /// Why the strongest distractor is wrong — a false claim, or a true
    /// statement that is not the best answer in context. Its anchors support
    /// the refutation, not the distractor's wording (contract §4).
    pub strongest_distractor: AnchoredClaim,
}
