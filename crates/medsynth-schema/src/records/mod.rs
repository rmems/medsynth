//! The three record types — one per format defined by the taxonomy (§3.3).

mod case_vignette;
mod mcq;
mod tutoring_example;

pub use case_vignette::{CaseVignetteRecord, VignetteQuestion};
pub use mcq::{McqOption, McqRationale, McqRecord};
pub use tutoring_example::{TutorResponse, TutoringExampleRecord, TutoringStyle};

use crate::error::ValidationError;
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

fn serde_plain<T: serde::Serialize>(v: T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}
