//! The three record types — one per format defined by the taxonomy (§3.3).

mod case_vignette;
mod mcq;
mod tutoring_example;

pub use case_vignette::{CaseVignetteRecord, VignetteQuestion};
pub use mcq::{McqOption, McqRationale, McqRecord, OptionLabel};
pub use tutoring_example::{TutorResponse, TutoringExampleRecord, TutoringStyle};

use crate::error::ValidationError;
use crate::provenance::{Provenance, VerificationStatus};
use crate::taxonomy::{Domain, Subdomain};

/// The domain/subdomain pairing rule from taxonomy §3.2, shared by every
/// record's `validate`.
pub(crate) fn check_taxonomy(
    errors: &mut Vec<ValidationError>,
    domain: Domain,
    subdomain: Subdomain,
) {
    if subdomain.parent_domain() != domain {
        errors.push(ValidationError::DomainSubdomainMismatch {
            domain: serde_plain(domain),
            subdomain: serde_plain(subdomain),
        });
    }
}

/// Every admitted record stores the contract version it was admitted under
/// (contract §9); drafts may leave it unset.
pub(crate) fn check_contract_version(
    errors: &mut Vec<ValidationError>,
    status: VerificationStatus,
    contract_version: Option<&str>,
) {
    if status == VerificationStatus::Admitted
        && contract_version.is_none_or(|v| v.trim().is_empty())
    {
        errors.push(ValidationError::AdmittedWithoutContractVersion);
    }
}

/// A model-drafted record must record the prompt version it was drafted
/// with, so the draft is reproducible.
pub(crate) fn check_provenance(errors: &mut Vec<ValidationError>, provenance: &Provenance) {
    if provenance.model.is_some()
        && provenance
            .prompt_version
            .as_deref()
            .is_none_or(|v| v.trim().is_empty())
    {
        errors.push(ValidationError::ModelWithoutPromptVersion);
    }
}

fn serde_plain<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .expect("taxonomy enums serialize to JSON")
        .as_str()
        .expect("taxonomy enums serialize to strings")
        .to_owned()
}
