//! The batch manifest.
//!
//! The record types have no seed, hash, rights-envelope, or receipt fields.
//! Those travel in this sidecar so generation never invents record fields.

use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::hash::{display_path, sha256_hex, sha256_str};
use crate::registry::SourceRegistry;
use crate::teacher::{PinLane, TeacherPin};

/// Manifest schema version.
pub const MANIFEST_VERSION: u32 = 1;

/// A referenced document and the state it was in for this batch.
#[derive(Debug, Clone, Serialize)]
pub struct DocumentRef {
    pub path: String,
    pub version: String,
    pub sha256: String,
}

/// Summary of every pin present in the pin registry.
#[derive(Debug, Clone, Serialize)]
pub struct PinSummary {
    pub pin_id: String,
    pub variant: String,
    pub lane: String,
}

/// Reference to the teacher pin registry as a whole.
#[derive(Debug, Clone, Serialize)]
pub struct TeachersRef {
    pub path: String,
    pub sha256: String,
    pub pins: Vec<PinSummary>,
}

/// Reference to the selected teacher pin.
#[derive(Debug, Clone, Serialize)]
pub struct TeacherPinRef {
    pub path: String,
    pub pin_id: String,
    pub variant: String,
    pub model_id: String,
    pub route: String,
    pub pin_entry_sha256: String,
    pub terms_document: String,
    pub terms_version: String,
    pub terms_date: String,
    pub terms_url: String,
    pub terms_clause: String,
    pub snapshot_date: String,
}

impl TeacherPinRef {
    pub fn from_pin(path: &Path, pin: &TeacherPin) -> Self {
        Self {
            path: display_path(path),
            pin_id: pin.pin_id.clone(),
            variant: pin.variant.clone(),
            model_id: pin.model_id.clone(),
            route: pin.route.clone(),
            pin_entry_sha256: pin.entry_sha256.clone(),
            terms_document: pin.terms_document.clone(),
            terms_version: pin.terms_version.clone(),
            terms_date: pin.terms_date.clone(),
            terms_url: pin.terms_url.clone(),
            terms_clause: pin.terms_clause.clone(),
            snapshot_date: pin.snapshot_date.clone(),
        }
    }
}

/// What happened to the teacher lane.
#[derive(Debug, Clone, Serialize)]
pub struct TeacherLaneReport {
    pub status: String,
    pub pin_id: Option<String>,
    pub error: Option<String>,
}

impl TeacherLaneReport {
    pub fn not_requested() -> Self {
        Self {
            status: "not_requested".to_owned(),
            pin_id: None,
            error: None,
        }
    }
}

/// Per-cell counts.
#[derive(Debug, Clone, Serialize)]
pub struct CellReport {
    pub domain: String,
    pub format: String,
    pub tier: String,
    pub subdomain: String,
    pub lane: String,
    pub matrix_target: u32,
    pub requested: u32,
    pub resolved_attempts: u32,
    pub in_flight_attempts: u32,
    pub budget_cap: u32,
    pub dispatched: u32,
    pub produced: u32,
    pub dropped: u32,
    pub budget_limited: bool,
    pub escalated: bool,
    pub status: String,
}

/// Batch totals.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Totals {
    pub requested: u32,
    pub dispatched: u32,
    pub produced: u32,
    pub dropped: u32,
}

/// One written record.
#[derive(Debug, Clone, Serialize)]
pub struct RecordEntry {
    pub id: String,
    /// The batch seed the record was generated under. Record ids hash the
    /// seed, but the manifest binds it directly so a record's provenance
    /// never requires recomputing the tag.
    pub seed: u64,
    pub format: String,
    pub domain: String,
    pub subdomain: String,
    pub tier: String,
    pub lane: String,
    pub generator: String,
    /// SHA-256 of the exact compact JSON written as the JSONL line.
    pub sha256: String,
    pub jsonl_file: String,
}

/// A draft that was not written.
#[derive(Debug, Clone, Serialize)]
pub struct DroppedDraft {
    pub id: Option<String>,
    pub format: String,
    pub domain: String,
    pub subdomain: String,
    pub tier: String,
    pub lane: String,
    pub stage: String,
    pub reasons: Vec<String>,
}

/// Model/vendor identity reported by the serving endpoint for one completion.
#[derive(Debug, Clone, Copy)]
pub struct ServingMetadata<'a> {
    pub model: Option<&'a str>,
    pub vendor: Option<&'a str>,
    pub session_file: Option<&'a str>,
    pub session_sha256: Option<&'a str>,
}

/// Rights envelope for one teacher draft.
#[derive(Debug, Clone, Serialize)]
pub struct RightsEnvelope {
    pub record_id: String,
    pub teacher_variant: String,
    pub model_id: String,
    /// The `model` value the endpoint actually returned, when it reported
    /// one. Pinned variants use dated names while providers answer with an
    /// alias; recording it keeps the two linked.
    pub served_model: Option<String>,
    /// Dispatch route named by the pin (for example Merge Gateway).
    pub route: String,
    /// Execution vendor that served the completion. For a gateway route this
    /// must come from routing metadata; for a direct route it is the pinned
    /// provider.
    pub serving_vendor: String,
    /// Hermes session-record file backing this draft, when the route uses Hermes.
    pub session_file: Option<String>,
    /// SHA-256 of the exact Hermes session-record file text.
    pub session_sha256: Option<String>,
    pub pin_id: String,
    /// Exact `docs/teachers.md` pin entry the output was generated under.
    pub pin_entry: String,
    pub pin_entry_sha256: String,
    /// SHA-256 of the exact record JSON.
    pub content_sha256: String,
}

impl RightsEnvelope {
    fn validate_pin(record_id: &str, pin: &TeacherPin) -> Result<(), String> {
        if record_id.trim().is_empty() {
            return Err("rights envelope: blank record id".to_owned());
        }
        if pin.variant.trim().is_empty() {
            return Err("rights envelope: blank teacher variant".to_owned());
        }
        if pin.pin_id.trim().is_empty() {
            return Err("rights envelope: blank pin id".to_owned());
        }
        if pin.route.trim().is_empty() {
            return Err("rights envelope: blank teacher route".to_owned());
        }
        if pin.entry_text.trim().is_empty() || pin.entry_sha256.trim().is_empty() {
            return Err("rights envelope: missing docs/teachers.md pin entry".to_owned());
        }
        Ok(())
    }

    fn serving_vendor(pin: &TeacherPin, serving: &ServingMetadata<'_>) -> Result<String, String> {
        if !pin.uses_merge_gateway() {
            return Ok(pin.provider.clone());
        }
        serving
            .vendor
            .filter(|vendor| !vendor.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                "rights envelope: Merge Gateway response lacks routing vendor metadata".to_owned()
            })
    }

    fn validate_hermes_session(
        pin: &TeacherPin,
        serving: &ServingMetadata<'_>,
    ) -> Result<(), String> {
        if !pin.route.to_ascii_lowercase().contains("hermes") {
            return Ok(());
        }
        for value in [serving.session_file, serving.session_sha256] {
            if value.is_none_or(|value| value.trim().is_empty()) {
                return Err(
                    "rights envelope: Hermes response lacks session-record provenance".to_owned(),
                );
            }
        }
        Ok(())
    }

    /// Build an envelope, or fail if any piece is missing. A draft whose
    /// envelope would be incomplete is dropped rather than written.
    pub fn build(
        record_id: &str,
        raw: &str,
        pin: &TeacherPin,
        serving: ServingMetadata<'_>,
    ) -> Result<Self, String> {
        Self::validate_pin(record_id, pin)?;
        let serving_vendor = Self::serving_vendor(pin, &serving)?;
        Self::validate_hermes_session(pin, &serving)?;
        Ok(Self {
            record_id: record_id.to_owned(),
            teacher_variant: pin.variant.clone(),
            model_id: pin.model_id.clone(),
            served_model: serving.model.map(str::to_owned),
            route: pin.route.clone(),
            serving_vendor,
            session_file: serving.session_file.map(str::to_owned),
            session_sha256: serving.session_sha256.map(str::to_owned),
            pin_id: pin.pin_id.clone(),
            pin_entry: pin.entry_text.clone(),
            pin_entry_sha256: pin.entry_sha256.clone(),
            content_sha256: sha256_str(raw),
        })
    }
}

/// One anchor receipt.
#[derive(Debug, Clone, Serialize)]
pub struct AnchorReceipt {
    pub record_id: String,
    pub source_id: String,
    pub locator: String,
    pub registry_entry_sha256: String,
    pub registry_version: String,
    pub occurrences: u32,
}

/// The complete manifest.
#[derive(Debug, Clone, Serialize)]
pub struct BatchManifest {
    pub manifest_version: u32,
    pub batch_id: String,
    pub seed: u64,
    pub config_sha256: String,
    pub config: Value,
    pub taxonomy: DocumentRef,
    pub sources: DocumentRef,
    /// The gold corpus used for anchor harvesting and few-shot exemplars, so
    /// a batch is reproducible against the exact exemplar inputs.
    pub gold: DocumentRef,
    pub teachers: TeachersRef,
    pub teacher_pin: Option<TeacherPinRef>,
    pub teacher_lane: TeacherLaneReport,
    pub cells: Vec<CellReport>,
    pub totals: Totals,
    pub records: Vec<RecordEntry>,
    pub dropped_by_self_validation: usize,
    pub dropped_by_generation: usize,
    pub dropped: Vec<DroppedDraft>,
    pub rights_envelopes: Vec<RightsEnvelope>,
    pub anchor_receipts: Vec<AnchorReceipt>,
}

impl BatchManifest {
    pub fn to_json_string(&self) -> Result<String, serde_json::Error> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }
}

/// The pinned inputs a batch id is derived from.
pub struct BatchIdInputs<'a> {
    pub seed: u64,
    pub config_sha256: &'a str,
    pub taxonomy_sha256: &'a str,
    pub sources_sha256: &'a str,
    /// SHA-256 of the gold corpus, so a batch id changes when the exemplar
    /// inputs that shaped its drafts change.
    pub gold_sha256: &'a str,
    pub pin_entry_sha256: Option<&'a str>,
}

/// Deterministic batch id: seed plus the pinned input documents.
pub fn batch_id(inputs: &BatchIdInputs<'_>) -> String {
    let material = format!(
        "v{MANIFEST_VERSION}\nseed={}\nconfig={}\ntaxonomy={}\nsources={}\ngold={}\npin={}\n",
        inputs.seed,
        inputs.config_sha256,
        inputs.taxonomy_sha256,
        inputs.sources_sha256,
        inputs.gold_sha256,
        inputs.pin_entry_sha256.unwrap_or("-")
    );
    format!("batch-{}", &sha256_hex(material.as_bytes())[..16])
}

/// Build the receipt for a record's anchors, deduplicated per record +
/// source + locator with an occurrence count.
pub fn anchor_receipts(
    record_id: &str,
    anchors: &[(&str, &str)],
    registry: &SourceRegistry,
) -> Result<Vec<AnchorReceipt>, String> {
    use std::collections::BTreeMap;
    let mut counts: BTreeMap<(String, String), u32> = BTreeMap::new();
    for (source_id, locator) in anchors {
        *counts
            .entry(((*source_id).to_owned(), (*locator).to_owned()))
            .or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|((source_id, locator), occurrences)| {
            let entry_sha256 = registry
                .entry_sha256(&source_id)
                .ok_or_else(|| format!("anchor source `{source_id}` is not in the registry"))?;
            Ok(AnchorReceipt {
                record_id: record_id.to_owned(),
                source_id,
                locator,
                registry_entry_sha256: entry_sha256.to_owned(),
                registry_version: registry.registry_version.clone(),
                occurrences,
            })
        })
        .collect()
}

/// A pin summary for the manifest.
pub fn pin_summary(pin: &TeacherPin) -> PinSummary {
    PinSummary {
        pin_id: pin.pin_id.clone(),
        variant: pin.variant.clone(),
        lane: match pin.lane {
            Some(PinLane::Teacher) => "teacher",
            Some(PinLane::ResearchOnly) => "research_only",
            None => "unspecified",
        }
        .to_owned(),
    }
}
