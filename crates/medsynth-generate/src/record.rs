//! A generated record in its format-specific `medsynth-schema` type.
//!
//! The record types are the source of truth; generation never adds fields.
//! Provenance, hashes, rights envelopes, and receipts live in the manifest,
//! not in invented record fields.

use medsynth_schema::{
    CaseVignetteRecord, Domain, McqRecord, Provenance, RecordId, Subdomain, Tier,
    TutoringExampleRecord, VerificationStatus,
};
use serde_json::Value;

use crate::config::TargetCell;
use crate::format::Format;
use crate::hash::sha256_str;
use crate::matrix::{domain_slug, tier_slug};

/// One draft, already typed.
#[derive(Debug, Clone)]
pub enum GeneratedRecord {
    Mcq(McqRecord),
    CaseVignette(CaseVignetteRecord),
    Tutoring(TutoringExampleRecord),
}

impl GeneratedRecord {
    pub fn format(&self) -> Format {
        match self {
            GeneratedRecord::Mcq(_) => Format::Mcq,
            GeneratedRecord::CaseVignette(_) => Format::CaseVignette,
            GeneratedRecord::Tutoring(_) => Format::TutoringExample,
        }
    }

    pub fn id(&self) -> &RecordId {
        match self {
            GeneratedRecord::Mcq(r) => &r.id,
            GeneratedRecord::CaseVignette(r) => &r.id,
            GeneratedRecord::Tutoring(r) => &r.id,
        }
    }

    pub fn domain(&self) -> Domain {
        match self {
            GeneratedRecord::Mcq(r) => r.domain,
            GeneratedRecord::CaseVignette(r) => r.domain,
            GeneratedRecord::Tutoring(r) => r.domain,
        }
    }

    pub fn subdomain(&self) -> Subdomain {
        match self {
            GeneratedRecord::Mcq(r) => r.subdomain,
            GeneratedRecord::CaseVignette(r) => r.subdomain,
            GeneratedRecord::Tutoring(r) => r.subdomain,
        }
    }

    pub fn tier(&self) -> Tier {
        match self {
            GeneratedRecord::Mcq(r) => r.tier,
            GeneratedRecord::CaseVignette(r) => r.tier,
            GeneratedRecord::Tutoring(r) => r.tier,
        }
    }

    pub fn provenance(&self) -> &Provenance {
        match self {
            GeneratedRecord::Mcq(r) => &r.provenance,
            GeneratedRecord::CaseVignette(r) => &r.provenance,
            GeneratedRecord::Tutoring(r) => &r.provenance,
        }
    }

    pub fn verification_status(&self) -> VerificationStatus {
        match self {
            GeneratedRecord::Mcq(r) => r.verification_status,
            GeneratedRecord::CaseVignette(r) => r.verification_status,
            GeneratedRecord::Tutoring(r) => r.verification_status,
        }
    }

    /// The exact compact JSON written as one JSONL line (no trailing
    /// newline). The manifest's content hash covers these bytes.
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        match self {
            GeneratedRecord::Mcq(r) => serde_json::to_string(r),
            GeneratedRecord::CaseVignette(r) => serde_json::to_string(r),
            GeneratedRecord::Tutoring(r) => serde_json::to_string(r),
        }
    }

    pub fn to_value(&self) -> Result<Value, serde_json::Error> {
        match self {
            GeneratedRecord::Mcq(r) => serde_json::to_value(r),
            GeneratedRecord::CaseVignette(r) => serde_json::to_value(r),
            GeneratedRecord::Tutoring(r) => serde_json::to_value(r),
        }
    }

    /// Every anchor the record carries, in document order, as
    /// `(source_id, locator)`.
    pub fn anchors(&self) -> Vec<(&str, &str)> {
        let mut out = Vec::new();
        match self {
            GeneratedRecord::Mcq(r) => {
                push_anchors(&mut out, &r.stem.anchors);
                for option in &r.options {
                    push_anchors(&mut out, &option.anchors);
                }
                push_anchors(&mut out, &r.rationale.key_explanation.anchors);
                push_anchors(&mut out, &r.rationale.strongest_distractor.anchors);
            }
            GeneratedRecord::CaseVignette(r) => {
                for question in &r.questions {
                    push_anchors(&mut out, &question.answer.anchors);
                }
                for step in &r.reasoning_chain {
                    push_anchors(&mut out, &step.anchors);
                }
                for point in &r.teaching_points {
                    push_anchors(&mut out, &point.anchors);
                }
            }
            GeneratedRecord::Tutoring(r) => {
                push_anchors(&mut out, &r.learner_prompt.anchors);
                push_anchors(&mut out, &r.tutor_response.anchors);
            }
        }
        out
    }
}

fn push_anchors<'a>(out: &mut Vec<(&'a str, &'a str)>, anchors: &'a [medsynth_schema::Anchor]) {
    for anchor in anchors {
        out.push((anchor.source_id.as_str(), anchor.locator.as_str()));
    }
}

/// Deterministic record id for a procedural or teacher draft.
///
/// The eight-hex seed tag keeps ids unique across seeds; the cell and index
/// keep them unique within a batch. Ids stay inside the schema's 64-character
/// slug limit for every taxonomy combination.
fn id_part(slug: &str) -> String {
    slug.replace('_', "-")
}

pub fn record_id_for(
    seed: u64,
    cell: &TargetCell,
    index: u32,
) -> Result<RecordId, medsynth_schema::InvalidRecordId> {
    let seed_tag = &sha256_str(&seed.to_string())[..8];
    let id = format!(
        "draft-{}-{}-{}-{}-{:03}",
        id_part(domain_slug(cell.domain)),
        id_part(cell.format.slug()),
        id_part(tier_slug(cell.tier)),
        seed_tag,
        index
    );
    RecordId::new(id)
}
