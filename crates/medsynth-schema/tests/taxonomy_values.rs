//! Guard the contract that the schema enums lift their values verbatim from
//! `docs/taxonomy.md` §3 — no synonyms, no re-spellings.

use medsynth_schema::{Domain, SourceId, Subdomain, Tier, VerificationStatus};

fn slugs<T: serde::Serialize>(values: &[T]) -> Vec<String> {
    values
        .iter()
        .map(|v| {
            serde_json::to_value(v)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

#[test]
fn domain_values_match_taxonomy() {
    use Domain::*;
    assert_eq!(
        slugs(&[Bbf, Cpf, Psb, PreClinical]),
        ["bbf", "cpf", "psb", "pre_clinical"]
    );
}

#[test]
fn subdomain_values_match_taxonomy() {
    use Subdomain::*;
    assert_eq!(
        slugs(&[
            BioPhysicalFoundations,
            Macromolecules,
            CellFunction,
            CellProcesses,
            SystemInteractions,
            AnimalStructure,
            Translation,
            PropertiesOfMatter,
            Transmission,
            Mechanics,
            Development,
            Socioeconomic,
            CultureIdentity,
            Psychology,
            Environment,
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
        ]),
        [
            "bio_physical_foundations",
            "macromolecules",
            "cell_function",
            "cell_processes",
            "system_interactions",
            "animal_structure",
            "translation",
            "properties_of_matter",
            "transmission",
            "mechanics",
            "development",
            "socioeconomic",
            "culture_identity",
            "psychology",
            "environment",
            "cardiovascular",
            "respiratory",
            "renal",
            "gastrointestinal",
            "endocrine",
            "neurology",
            "musculoskeletal",
            "hematology_oncology",
            "infectious_disease",
            "pharmacology_principles",
        ]
    );
}

#[test]
fn subdomain_parent_mapping_matches_taxonomy() {
    use Domain::*;
    use Subdomain::*;
    for (subdomain, domain) in [
        (BioPhysicalFoundations, Bbf),
        (Macromolecules, Bbf),
        (CellFunction, Bbf),
        (CellProcesses, Bbf),
        (SystemInteractions, Bbf),
        (AnimalStructure, Bbf),
        (Translation, Cpf),
        (PropertiesOfMatter, Cpf),
        (Transmission, Cpf),
        (Mechanics, Cpf),
        (Development, Psb),
        (Socioeconomic, Psb),
        (CultureIdentity, Psb),
        (Psychology, Psb),
        (Environment, Psb),
        (Cardiovascular, PreClinical),
        (Respiratory, PreClinical),
        (Renal, PreClinical),
        (Gastrointestinal, PreClinical),
        (Endocrine, PreClinical),
        (Neurology, PreClinical),
        (Musculoskeletal, PreClinical),
        (HematologyOncology, PreClinical),
        (InfectiousDisease, PreClinical),
        (PharmacologyPrinciples, PreClinical),
    ] {
        assert_eq!(subdomain.parent_domain(), domain, "{subdomain:?}");
    }
}

#[test]
fn tier_values_match_taxonomy() {
    use Tier::*;
    assert_eq!(
        slugs(&[Foundation, Application, Reasoning]),
        ["foundation", "application", "reasoning"]
    );
}

#[test]
fn verification_status_values_match_contract() {
    use VerificationStatus::*;
    assert_eq!(
        slugs(&[Unverified, KeyChecked, AnchorChecked, Admitted]),
        ["unverified", "key-checked", "anchor-checked", "admitted"]
    );
}

#[test]
fn source_id_pattern() {
    assert!(SourceId::is_valid("example-biochemistry-9e"));
    assert!(SourceId::is_valid("abc"));
    assert!(!SourceId::is_valid("ab"));
    assert!(!SourceId::is_valid("A-textbook"));
    assert!(!SourceId::is_valid("has space"));
    assert!(!SourceId::is_valid("-leading"));
    assert!(!SourceId::is_valid("trailing-"));
    assert!(!SourceId::is_valid(""));
    assert!(serde_json::from_str::<SourceId>("\"bad id\"").is_err());
}
