//! Cross-field record validation errors — the rules JSON Schema cannot
//! express. Returned by each record type's `validate` method.

/// A cross-field consistency violation on a record.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValidationError {
    /// `subdomain` does not belong to `domain` per the taxonomy §3.2 mapping.
    #[error("subdomain `{subdomain}` does not belong to domain `{domain}`")]
    DomainSubdomainMismatch { domain: String, subdomain: String },
    /// The MCQ `key` matches no option label.
    #[error("key `{key}` does not match any option label")]
    KeyMatchesNoOption { key: String },
    /// Two options share a label.
    #[error("duplicate option label `{label}`")]
    DuplicateOptionLabel { label: String },
}
