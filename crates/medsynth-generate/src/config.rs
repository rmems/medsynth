//! Run configuration: the target list plus seed and optional teacher
//! selection. The target list is checked against the taxonomy §5 matrix.

use std::collections::BTreeSet;
use std::path::Path;

use medsynth_schema::{Domain, Subdomain, Tier};
use serde::{Deserialize, Serialize};

use crate::error::GenerateError;
use crate::format::Format;
use crate::matrix::{CellKey, CoverageMatrix, domain_slug};

/// Which lane a target cell uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    /// Deterministic template generation (templatable MCQs).
    Procedural,
    /// Pinned teacher-model drafting (vignettes, tutoring, non-templatable
    /// MCQs).
    Teacher,
}

/// One target cell from the run config.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetCell {
    pub domain: Domain,
    pub format: Format,
    pub tier: Tier,
    /// Drafts requested for this cell. Must not exceed the matrix target.
    pub count: u32,
    pub lane: Lane,
    /// Procedural template id; required for procedural cells and forbidden
    /// for teacher cells.
    #[serde(default)]
    pub template: Option<String>,
    pub subdomain: Subdomain,
    /// Draft attempts already resolved for this cell outside this batch.
    #[serde(default)]
    pub resolved_attempts: u32,
    /// Draft attempts already in flight for this cell outside this batch.
    /// Together with `resolved_attempts` and this batch's dispatch, the cell
    /// must stay within 3× the matrix target (taxonomy §5.2).
    #[serde(default)]
    pub in_flight_attempts: u32,
}

impl TargetCell {
    pub fn key(&self) -> CellKey {
        CellKey {
            domain: self.domain,
            format: self.format,
            tier: self.tier,
        }
    }

    pub fn label(&self) -> String {
        self.key().label()
    }

    pub fn subdomain_slug(&self) -> &'static str {
        subdomain_slug(self.subdomain)
    }
}

/// The batch configuration file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    pub seed: u64,
    pub cells: Vec<TargetCell>,
    #[serde(default)]
    pub teacher: Option<TeacherSelection>,
}

/// Which pinned teacher to use and how to budget its prompts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeacherSelection {
    pub pin_id: String,
    #[serde(default = "default_prompt_version")]
    pub prompt_version: String,
    #[serde(default = "default_max_prompt_tokens")]
    pub max_prompt_tokens: u32,
    #[serde(default = "default_max_output_tokens")]
    pub max_output_tokens: u32,
    #[serde(default = "default_exemplars")]
    pub exemplars_per_prompt: usize,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
}

impl Default for TeacherSelection {
    fn default() -> Self {
        Self {
            pin_id: String::new(),
            prompt_version: default_prompt_version(),
            max_prompt_tokens: default_max_prompt_tokens(),
            max_output_tokens: default_max_output_tokens(),
            exemplars_per_prompt: default_exemplars(),
            temperature: default_temperature(),
        }
    }
}

fn default_prompt_version() -> String {
    "teacher-draft-v1".to_owned()
}

const fn default_max_prompt_tokens() -> u32 {
    6000
}

const fn default_max_output_tokens() -> u32 {
    1200
}

const fn default_exemplars() -> usize {
    1
}

const fn default_temperature() -> f32 {
    0.2
}

impl RunConfig {
    /// Load a config file.
    pub fn load(path: &Path) -> Result<Self, GenerateError> {
        let text =
            std::fs::read_to_string(path).map_err(|e| GenerateError::io(path.to_path_buf(), e))?;
        serde_json::from_str(&text)
            .map_err(|e| GenerateError::Config(format!("{}: {e}", path.display())))
    }

    /// Check the target list against the matrix and the lane rules.
    pub fn validate_against(&self, matrix: &CoverageMatrix) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        let mut seen = BTreeSet::new();
        for cell in &self.cells {
            if !seen.insert(cell.key()) {
                errors.push(format!("duplicate target cell `{}`", cell.label()));
            }
            cell.check_shape(&mut errors);
            cell.check_matrix(matrix, &mut errors);
            cell.check_lane(&mut errors);
        }
        self.check_teacher(&mut errors);
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    fn check_teacher(&self, errors: &mut Vec<String>) {
        let needs_teacher = self.cells.iter().any(|c| c.lane == Lane::Teacher);
        if needs_teacher && self.teacher.is_none() {
            errors.push("teacher cells are configured but `teacher` is absent".to_owned());
        }
        if let Some(teacher) = &self.teacher {
            teacher.check(errors);
        }
    }
}

impl TargetCell {
    fn check_shape(&self, errors: &mut Vec<String>) {
        if self.subdomain.parent_domain() != self.domain {
            errors.push(format!(
                "cell `{}`: subdomain `{}` does not belong to domain `{}`",
                self.label(),
                self.subdomain_slug(),
                domain_slug(self.domain)
            ));
        }
        if self.count == 0 {
            errors.push(format!("cell `{}`: count must be positive", self.label()));
        }
    }

    fn check_matrix(&self, matrix: &CoverageMatrix, errors: &mut Vec<String>) {
        match matrix.target(self.key()) {
            Some(target) => {
                // The matrix target is admitted fill, not a draft ceiling:
                // rejected drafts may be replaced up to 3× the target. A
                // request larger than the whole 3× budget can never be
                // dispatched, so reject it rather than silently truncating
                // most of it.
                let cap = target.saturating_mul(3);
                if self.count > cap {
                    errors.push(format!(
                        "cell `{}`: requested {} drafts exceeds the 3× budget cap {cap}",
                        self.label(),
                        self.count
                    ));
                }
            }
            None => errors.push(format!(
                "cell `{}` is not in the taxonomy §5 matrix",
                self.label()
            )),
        }
    }

    fn check_lane(&self, errors: &mut Vec<String>) {
        match self.lane {
            Lane::Procedural => {
                if !self.format.procedural() {
                    errors.push(format!(
                        "cell `{}`: procedural lane only produces `mcq` drafts",
                        self.label()
                    ));
                }
                if self.template.as_deref().is_none_or(str::is_empty) {
                    errors.push(format!(
                        "cell `{}`: procedural lane needs a template id",
                        self.label()
                    ));
                }
            }
            Lane::Teacher => {
                if self.template.is_some() {
                    errors.push(format!(
                        "cell `{}`: teacher lane must not set a procedural template",
                        self.label()
                    ));
                }
            }
        }
    }
}

impl TeacherSelection {
    fn check(&self, errors: &mut Vec<String>) {
        if self.pin_id.trim().is_empty() {
            errors.push("teacher.pin_id must not be blank".to_owned());
        }
        if self.prompt_version.trim().is_empty() {
            errors.push("teacher.prompt_version must not be blank".to_owned());
        }
        if self.max_prompt_tokens == 0 || self.max_output_tokens == 0 {
            errors.push("teacher token budgets must be positive".to_owned());
        }
        if !(0.0..=2.0).contains(&self.temperature) {
            errors.push("teacher.temperature must be between 0 and 2".to_owned());
        }
    }
}

pub(crate) fn subdomain_slug(subdomain: Subdomain) -> &'static str {
    match subdomain {
        Subdomain::BioPhysicalFoundations => "bio_physical_foundations",
        Subdomain::Macromolecules => "macromolecules",
        Subdomain::CellFunction => "cell_function",
        Subdomain::CellProcesses => "cell_processes",
        Subdomain::SystemInteractions => "system_interactions",
        Subdomain::AnimalStructure => "animal_structure",
        Subdomain::Translation => "translation",
        Subdomain::PropertiesOfMatter => "properties_of_matter",
        Subdomain::Transmission => "transmission",
        Subdomain::Mechanics => "mechanics",
        Subdomain::Development => "development",
        Subdomain::Socioeconomic => "socioeconomic",
        Subdomain::CultureIdentity => "culture_identity",
        Subdomain::Psychology => "psychology",
        Subdomain::Environment => "environment",
        Subdomain::Cardiovascular => "cardiovascular",
        Subdomain::Respiratory => "respiratory",
        Subdomain::Renal => "renal",
        Subdomain::Gastrointestinal => "gastrointestinal",
        Subdomain::Endocrine => "endocrine",
        Subdomain::Neurology => "neurology",
        Subdomain::Musculoskeletal => "musculoskeletal",
        Subdomain::HematologyOncology => "hematology_oncology",
        Subdomain::InfectiousDisease => "infectious_disease",
        Subdomain::PharmacologyPrinciples => "pharmacology_principles",
    }
}
