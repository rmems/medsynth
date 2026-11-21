//! Self-validation before a draft is written.
//!
//! Three gates, in order:
//! 1. JSON Schema validation against `schemas/<format>.v1.json`;
//! 2. deserialization into the `medsynth-schema` record type and its
//!    cross-field `validate()`;
//! 3. generation invariants — unverified status, taxonomy/contract versions,
//!    lane-appropriate provenance, registry-resolved anchors, and (for teacher
//!    drafts) the cell's anchor allowlist.
//!
//! A draft that fails any gate is dropped and counted in the manifest; it is
//! never written.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use medsynth_schema::{
    CURRENT_SCHEMA_VERSION, CaseVignetteRecord, McqRecord, Provenance, TutoringExampleRecord,
    VerificationStatus,
};
use serde_json::Value;

use crate::error::GenerateError;
use crate::format::Format;
use crate::record::GeneratedRecord;
use crate::registry::SourceRegistry;
use crate::teacher::TeacherPin;

/// What provenance the lane expects.
pub enum ExpectedProvenance<'a> {
    /// A procedural template record.
    Procedural,
    /// A teacher record generated under this pin and prompt version.
    Teacher {
        pin: &'a TeacherPin,
        prompt_version: &'a str,
    },
}

/// Context for self-validation.
pub struct ValidationContext<'a> {
    pub expected: ExpectedProvenance<'a>,
    pub registry: &'a SourceRegistry,
    /// Teacher drafts may cite only the anchor pairs supplied in the prompt.
    pub anchor_allowlist: Option<&'a BTreeSet<(String, String)>>,
}

/// Compiled validators for the three formats.
pub struct DraftValidator {
    validators: BTreeMap<Format, jsonschema::Validator>,
    schemas_dir: PathBuf,
}

impl DraftValidator {
    /// Load and compile the checked-in schemas.
    pub fn load(schemas_dir: &Path) -> Result<Self, GenerateError> {
        let mut validators = BTreeMap::new();
        for format in Format::all() {
            let path =
                schemas_dir.join(format!("{}.v{CURRENT_SCHEMA_VERSION}.json", format.slug()));
            let text =
                std::fs::read_to_string(&path).map_err(|e| GenerateError::io(path.clone(), e))?;
            let schema: Value = serde_json::from_str(&text).map_err(|e| {
                GenerateError::Schema(format!("{}: invalid schema JSON: {e}", path.display()))
            })?;
            let validator = jsonschema::validator_for(&schema).map_err(|e| {
                GenerateError::Schema(format!("{}: cannot compile schema: {e}", path.display()))
            })?;
            validators.insert(format, validator);
        }
        Ok(Self {
            validators,
            schemas_dir: schemas_dir.to_path_buf(),
        })
    }

    pub fn schemas_dir(&self) -> &Path {
        &self.schemas_dir
    }

    /// Validate one draft's exact JSON text.
    pub fn validate_raw(
        &self,
        format: Format,
        raw: &str,
        context: &ValidationContext<'_>,
    ) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let value: Value = match serde_json::from_str(raw) {
            Ok(value) => value,
            Err(e) => return Err(vec![format!("invalid JSON: {e}")]),
        };
        let validator = self
            .validators
            .get(&format)
            .ok_or_else(|| vec![format!("no validator for format `{format}`")])?;
        for error in validator.iter_errors(&value) {
            errors.push(format!("{}: {error}", error.instance_path()));
        }
        if !errors.is_empty() {
            return Err(errors);
        }

        let generated = parse_typed(format, raw)?;
        errors.extend(cross_validate(&generated));
        errors.extend(check_invariants(&generated, context));
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// Deserialize the draft into its typed `medsynth-schema` record.
fn parse_typed(format: Format, raw: &str) -> Result<GeneratedRecord, Vec<String>> {
    match format {
        Format::Mcq => serde_json::from_str::<McqRecord>(raw)
            .map(GeneratedRecord::Mcq)
            .map_err(|e| vec![format!("typed mcq: {e}")]),
        Format::CaseVignette => serde_json::from_str::<CaseVignetteRecord>(raw)
            .map(GeneratedRecord::CaseVignette)
            .map_err(|e| vec![format!("typed case_vignette: {e}")]),
        Format::TutoringExample => serde_json::from_str::<TutoringExampleRecord>(raw)
            .map(GeneratedRecord::Tutoring)
            .map_err(|e| vec![format!("typed tutoring_example: {e}")]),
    }
}

/// The record type's own cross-field checks.
fn cross_validate(record: &GeneratedRecord) -> Vec<String> {
    let result = match record {
        GeneratedRecord::Mcq(record) => record.validate(),
        GeneratedRecord::CaseVignette(record) => record.validate(),
        GeneratedRecord::Tutoring(record) => record.validate(),
    };
    result
        .err()
        .map(|cross| cross.iter().map(ToString::to_string).collect())
        .unwrap_or_default()
}

fn check_invariants(record: &GeneratedRecord, context: &ValidationContext<'_>) -> Vec<String> {
    let mut errors = Vec::new();
    if record.verification_status() != VerificationStatus::Unverified {
        errors.push("generation must leave verification_status unverified".to_owned());
    }
    let provenance = record.provenance();
    if provenance.generator.trim().is_empty() {
        errors.push("provenance.generator is blank".to_owned());
    }
    match &context.expected {
        ExpectedProvenance::Procedural => {
            check_procedural_provenance(provenance, &mut errors);
        }
        ExpectedProvenance::Teacher {
            pin,
            prompt_version,
        } => {
            check_teacher_provenance(provenance, pin, prompt_version, &mut errors);
        }
    }
    check_anchors(record, context, &mut errors);
    if !taxonomy_versions_ok(record) {
        errors.push(format!(
            "taxonomy_version must be `{}` and contract_version must be `{}`",
            crate::TAXONOMY_VERSION,
            crate::CONTRACT_VERSION
        ));
    }
    errors
}

/// Anchor receipts: locators must be non-blank and registry-resolved; teacher
/// drafts may only cite the cell's approved anchor list.
fn check_anchors(
    record: &GeneratedRecord,
    context: &ValidationContext<'_>,
    errors: &mut Vec<String>,
) {
    let mut seen = BTreeSet::new();
    for (source_id, locator) in record.anchors() {
        if locator.trim().is_empty() {
            errors.push(format!("anchor `{source_id}` has a blank locator"));
        }
        if !context.registry.contains(source_id) {
            errors.push(format!(
                "anchor cites `{source_id}`, which is not in docs/sources.json"
            ));
        }
        if let Some(allowlist) = context.anchor_allowlist
            && !allowlist.contains(&(source_id.to_owned(), locator.to_owned()))
            && seen.insert((source_id.to_owned(), locator.to_owned()))
        {
            errors.push(format!(
                "teacher anchor `{source_id}` / `{locator}` is not in the cell's approved anchor list"
            ));
        }
    }
}

fn taxonomy_versions_ok(record: &GeneratedRecord) -> bool {
    let check = |taxonomy: &str, contract: Option<&str>| {
        taxonomy == crate::TAXONOMY_VERSION && contract == Some(crate::CONTRACT_VERSION)
    };
    match record {
        GeneratedRecord::Mcq(r) => check(&r.taxonomy_version, r.contract_version.as_deref()),
        GeneratedRecord::CaseVignette(r) => {
            check(&r.taxonomy_version, r.contract_version.as_deref())
        }
        GeneratedRecord::Tutoring(r) => check(&r.taxonomy_version, r.contract_version.as_deref()),
    }
}

fn check_procedural_provenance(provenance: &Provenance, errors: &mut Vec<String>) {
    if provenance
        .template_version
        .as_deref()
        .is_none_or(|v| v.trim().is_empty())
    {
        errors.push("procedural draft has no template_version".to_owned());
    }
    if provenance
        .generator
        .strip_prefix("template:")
        .is_none_or(|g| g.trim().is_empty())
    {
        errors.push("procedural draft's generator must start with `template:`".to_owned());
    }
    if provenance.model.is_some() || provenance.prompt_version.is_some() {
        errors.push("procedural draft must not carry model or prompt_version".to_owned());
    }
}

fn check_teacher_provenance(
    provenance: &Provenance,
    pin: &TeacherPin,
    prompt_version: &str,
    errors: &mut Vec<String>,
) {
    if provenance.generator != "teacher-pipeline-v1" {
        errors.push("teacher draft's generator must be `teacher-pipeline-v1`".to_owned());
    }
    if provenance.model.as_deref() != Some(pin.variant.as_str()) {
        errors.push(format!(
            "teacher draft's model must be pinned variant `{}`",
            pin.variant
        ));
    }
    if provenance.prompt_version.as_deref() != Some(prompt_version) {
        errors.push(format!(
            "teacher draft's prompt_version must be `{prompt_version}`"
        ));
    }
    if provenance.template_version.is_some() {
        errors.push("teacher draft must not carry template_version".to_owned());
    }
}
