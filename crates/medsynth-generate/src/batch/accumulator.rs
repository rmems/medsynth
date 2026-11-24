//! Batch-wide record and drop accounting.
//!
//! Both lanes feed this accumulator so accepting or dropping a draft updates
//! every related collection and counter in one place.

use crate::config::{Lane, TargetCell};
use crate::error::GenerateError;
use crate::manifest::{
    AnchorReceipt, DroppedDraft, RecordEntry, RightsEnvelope, anchor_receipts,
};
use crate::matrix::{domain_slug, tier_slug};
use crate::record::GeneratedRecord;
use crate::registry::SourceRegistry;

use super::OutputRecord;

/// A draft that passed self-validation and is ready to be written.
pub(crate) struct AcceptedDraft {
    pub record: GeneratedRecord,
    pub raw: String,
    pub lane: Lane,
    pub envelope: Option<RightsEnvelope>,
}

/// The reason a draft was dropped, with the counters it affects.
pub(crate) enum DropStage {
    Generation(Vec<String>),
    SelfValidation(Vec<String>),
    TeacherTransport(String),
    TeacherBudget(String),
    TeacherParse(String),
    RightsEnvelope(String),
}

impl DropStage {
    fn label(&self) -> &'static str {
        match self {
            Self::Generation(_) => "generation",
            Self::SelfValidation(_) => "self_validation",
            Self::TeacherTransport(_) => "teacher_transport",
            Self::TeacherBudget(_) => "teacher_budget",
            Self::TeacherParse(_) => "teacher_parse",
            Self::RightsEnvelope(_) => "rights_envelope",
        }
    }

    fn reasons(self) -> Vec<String> {
        match self {
            Self::Generation(reasons)
            | Self::SelfValidation(reasons) => reasons,
            Self::TeacherTransport(reason)
            | Self::TeacherBudget(reason)
            | Self::TeacherParse(reason)
            | Self::RightsEnvelope(reason) => vec![reason],
        }
    }

    fn counts_as_self_validation(&self) -> bool {
        matches!(self, Self::SelfValidation(_) | Self::RightsEnvelope(_))
    }
}

/// A failed draft and the identity to attach to the manifest entry.
pub(crate) struct DraftFailure {
    pub id: Option<String>,
    pub stage: DropStage,
}

impl DraftFailure {
    pub fn new(id: Option<String>, stage: DropStage) -> Self {
        Self { id, stage }
    }
}

#[derive(Default)]
pub(crate) struct BatchAccumulator {
    pub records: Vec<OutputRecord>,
    pub record_entries: Vec<RecordEntry>,
    pub receipts: Vec<AnchorReceipt>,
    pub envelopes: Vec<RightsEnvelope>,
    pub dropped: Vec<DroppedDraft>,
    pub dropped_self: u32,
    pub dropped_generation: u32,
}

impl BatchAccumulator {
    pub fn accept(
        &mut self,
        draft: AcceptedDraft,
        registry: &SourceRegistry,
    ) -> Result<(), GenerateError> {
        let id = draft.record.id().as_str().to_owned();
        let record_receipts = anchor_receipts(&id, &draft.record.anchors(), registry)
            .map_err(|reason| GenerateError::Other(format!("record `{id}`: {reason}")))?;
        self.record_entries.push(RecordEntry {
            id,
            format: draft.record.format().slug().to_owned(),
            domain: domain_slug(draft.record.domain()).to_owned(),
            subdomain: crate::config::subdomain_slug(draft.record.subdomain()).to_owned(),
            tier: tier_slug(draft.record.tier()).to_owned(),
            lane: match draft.lane {
                Lane::Procedural => "procedural",
                Lane::Teacher => "teacher",
            }
            .to_owned(),
            generator: draft.record.provenance().generator.clone(),
            sha256: crate::hash::sha256_str(&draft.raw),
            jsonl_file: format!("{}.jsonl", draft.record.format().slug()),
        });
        self.receipts.extend(record_receipts);
        if let Some(envelope) = draft.envelope {
            self.envelopes.push(envelope);
        }
        self.records.push(OutputRecord {
            record: draft.record,
            raw: draft.raw,
            lane: draft.lane,
        });
        Ok(())
    }

    pub fn drop_draft(&mut self, cell: &TargetCell, lane: Lane, id: Option<String>, stage: DropStage) {
        if stage.counts_as_self_validation() {
            self.dropped_self += 1;
        } else {
            self.dropped_generation += 1;
        }
        self.dropped.push(DroppedDraft {
            id,
            format: cell.format.slug().to_owned(),
            domain: domain_slug(cell.domain).to_owned(),
            subdomain: cell.subdomain_slug().to_owned(),
            tier: tier_slug(cell.tier).to_owned(),
            lane: match lane {
                Lane::Procedural => "procedural",
                Lane::Teacher => "teacher",
            }
            .to_owned(),
            stage: stage.label().to_owned(),
            reasons: stage.reasons(),
        });
    }
}
