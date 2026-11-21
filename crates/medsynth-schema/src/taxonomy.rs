//! Taxonomy value sets, lifted verbatim from `docs/taxonomy.md` §3.
//!
//! These enums are the serialized vocabulary records classify under. Do not
//! add, rename, or re-spell variants here: new slugs are added to
//! `docs/taxonomy.md` first, then mirrored here (taxonomy §6 amendment rules).

use schemars::JsonSchema;
use serde::Serialize;

/// `domain` — the four taxonomy domains (taxonomy §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Domain {
    /// Biological & Biochemical Foundations (MCAT).
    Bbf,
    /// Chemical & Physical Foundations (MCAT).
    Cpf,
    /// Psychological, Social & Biological Foundations (MCAT).
    Psb,
    /// Pre-clinical systems.
    PreClinical,
}

string_enum! {
    Domain,
    "bbf" => Domain::Bbf,
    "cpf" => Domain::Cpf,
    "psb" => Domain::Psb,
    "pre_clinical" => Domain::PreClinical,
}

/// `subdomain` — the twenty-five taxonomy subdomain slugs (taxonomy §3.2).
///
/// Every subdomain has exactly one parent domain; the mapping is enforced by
/// [`Subdomain::parent_domain`] and checked by each record's `validate`
/// method, since the domain/subdomain pairing cannot be expressed in JSON
/// Schema alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Subdomain {
    // domain: bbf
    /// Chemical & physical foundations of biological systems.
    BioPhysicalFoundations,
    /// Structure and function of biological macromolecules.
    Macromolecules,
    /// Principles of cell function.
    CellFunction,
    /// Processes governing cellular organization and function.
    CellProcesses,
    /// Interactions between systems and organs.
    SystemInteractions,
    /// Unique aspects of animal structure and function.
    AnimalStructure,
    // domain: cpf
    /// Translation of chemical and physical sciences.
    Translation,
    /// Properties of matter.
    PropertiesOfMatter,
    /// Principles of transmission of information.
    Transmission,
    /// Principles of mechanics.
    Mechanics,
    // domain: psb
    /// Development and lifespan learning processes.
    Development,
    /// Socioeconomic status, education, and poverty.
    Socioeconomic,
    /// Culture, language, and identity.
    CultureIdentity,
    /// Psychology and social behaviors.
    Psychology,
    /// Environment and external factors.
    Environment,
    // domain: pre_clinical
    Cardiovascular,
    Respiratory,
    Renal,
    Gastrointestinal,
    Endocrine,
    Neurology,
    Musculoskeletal,
    HematologyOncology,
    InfectiousDisease,
    PharmacologyPrinciples,
}

impl Subdomain {
    /// All subdomain variants, in taxonomy §3.2 order.
    pub const ALL: [Subdomain; 25] = [
        Subdomain::BioPhysicalFoundations,
        Subdomain::Macromolecules,
        Subdomain::CellFunction,
        Subdomain::CellProcesses,
        Subdomain::SystemInteractions,
        Subdomain::AnimalStructure,
        Subdomain::Translation,
        Subdomain::PropertiesOfMatter,
        Subdomain::Transmission,
        Subdomain::Mechanics,
        Subdomain::Development,
        Subdomain::Socioeconomic,
        Subdomain::CultureIdentity,
        Subdomain::Psychology,
        Subdomain::Environment,
        Subdomain::Cardiovascular,
        Subdomain::Respiratory,
        Subdomain::Renal,
        Subdomain::Gastrointestinal,
        Subdomain::Endocrine,
        Subdomain::Neurology,
        Subdomain::Musculoskeletal,
        Subdomain::HematologyOncology,
        Subdomain::InfectiousDisease,
        Subdomain::PharmacologyPrinciples,
    ];

    /// The parent `domain` of this subdomain (taxonomy §3.2 mapping table).
    pub fn parent_domain(self) -> Domain {
        use Subdomain::*;
        match self {
            BioPhysicalFoundations
            | Macromolecules
            | CellFunction
            | CellProcesses
            | SystemInteractions
            | AnimalStructure => Domain::Bbf,
            Translation | PropertiesOfMatter | Transmission | Mechanics => Domain::Cpf,
            Development | Socioeconomic | CultureIdentity | Psychology | Environment => Domain::Psb,
            Cardiovascular
            | Respiratory
            | Renal
            | Gastrointestinal
            | Endocrine
            | Neurology
            | Musculoskeletal
            | HematologyOncology
            | InfectiousDisease
            | PharmacologyPrinciples => Domain::PreClinical,
        }
    }
}

string_enum! {
    Subdomain,
    "bio_physical_foundations" => Subdomain::BioPhysicalFoundations,
    "macromolecules" => Subdomain::Macromolecules,
    "cell_function" => Subdomain::CellFunction,
    "cell_processes" => Subdomain::CellProcesses,
    "system_interactions" => Subdomain::SystemInteractions,
    "animal_structure" => Subdomain::AnimalStructure,
    "translation" => Subdomain::Translation,
    "properties_of_matter" => Subdomain::PropertiesOfMatter,
    "transmission" => Subdomain::Transmission,
    "mechanics" => Subdomain::Mechanics,
    "development" => Subdomain::Development,
    "socioeconomic" => Subdomain::Socioeconomic,
    "culture_identity" => Subdomain::CultureIdentity,
    "psychology" => Subdomain::Psychology,
    "environment" => Subdomain::Environment,
    "cardiovascular" => Subdomain::Cardiovascular,
    "respiratory" => Subdomain::Respiratory,
    "renal" => Subdomain::Renal,
    "gastrointestinal" => Subdomain::Gastrointestinal,
    "endocrine" => Subdomain::Endocrine,
    "neurology" => Subdomain::Neurology,
    "musculoskeletal" => Subdomain::Musculoskeletal,
    "hematology_oncology" => Subdomain::HematologyOncology,
    "infectious_disease" => Subdomain::InfectiousDisease,
    "pharmacology_principles" => Subdomain::PharmacologyPrinciples,
}

/// `tier` — difficulty tiers, defined per format (taxonomy §3.3–§4). The same
/// tier name means different things in an MCQ, a case vignette, and a tutoring
/// example; a record's tier is always interpreted relative to its format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Foundation,
    Application,
    Reasoning,
}

string_enum! {
    Tier,
    "foundation" => Tier::Foundation,
    "application" => Tier::Application,
    "reasoning" => Tier::Reasoning,
}
