//! The case vignette record (format `case_vignette`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::ValidationError;
use crate::id::RecordId;
use crate::labels::RecordLabel;
use crate::provenance::{Provenance, VerificationStatus};
use crate::taxonomy::{Domain, Subdomain, Tier};
use crate::text::{AnchoredClaim, AnchoredText};
use crate::version::SchemaVersion;

/// A clinical-style vignette about a fictional patient, with associated
/// questions (taxonomy §3.3 `case_vignette`, tier definitions §4.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseVignetteRecord {
    /// Version of this record schema.
    pub schema_version: SchemaVersion,
    /// Unique record identifier.
    pub id: RecordId,
    /// Taxonomy domain (taxonomy §3.1).
    pub domain: Domain,
    /// Taxonomy subdomain; must belong to `domain` (taxonomy §3.2). Checked
    /// by [`CaseVignetteRecord::validate`].
    pub subdomain: Subdomain,
    /// Difficulty tier, interpreted per the `case_vignette` definitions
    /// (taxonomy §4.2).
    pub tier: Tier,
    /// Presentation of the fictional patient (contract §2): the narrative a
    /// learner reads first. Invented observations are exempt from anchoring
    /// (contract §4); claims *about* them live in `reasoning_chain` and
    /// `teaching_points`, where anchors attach.
    #[schemars(length(min = 1))]
    pub presentation: String,
    /// Objective findings — vitals, labs, imaging, exam findings — invented
    /// for this fictional patient. These are observations, not claims, and
    /// are exempt from anchoring (contract §4); interpretive claims about
    /// them belong in `reasoning_chain`.
    pub findings: Vec<String>,
    /// Questions posed about the vignette, each with its expected answer.
    #[schemars(length(min = 1))]
    pub questions: Vec<VignetteQuestion>,
    /// Stepwise reasoning chain connecting the findings to the answers —
    /// the "how you get there" a learner should be able to reproduce.
    #[schemars(length(min = 1))]
    pub reasoning_chain: Vec<AnchoredText>,
    /// Teaching points the vignette is designed to convey; each is a claim
    /// and must carry at least one anchor (contract §4).
    #[schemars(length(min = 1))]
    pub teaching_points: Vec<AnchoredClaim>,
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
    /// [`CaseVignetteRecord::validate`].
    pub contract_version: Option<String>,
}

impl CaseVignetteRecord {
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

/// One question on the vignette and the answer a learner should reach.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VignetteQuestion {
    /// The question posed to the learner about the fictional patient.
    #[schemars(length(min = 1))]
    pub question: String,
    /// The expected answer, with anchors supporting it.
    pub answer: AnchoredClaim,
}
