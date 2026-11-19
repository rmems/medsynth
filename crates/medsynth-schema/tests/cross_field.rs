//! Regression tests for the cross-field rules that live in each record's
//! `validate`, beyond what JSON Schema expresses.

use std::path::{Path, PathBuf};

use medsynth_schema::{McqRecord, SchemaVersion, ValidationError};

fn fixture(format_dir: &str, name: &str) -> serde_json::Value {
    let path: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("fixtures")
        .join(format_dir)
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn mcq() -> McqRecord {
    serde_json::from_value(fixture("mcq", "valid-enzyme-kinetics.json")).unwrap()
}

#[test]
fn admitted_requires_contract_version() {
    let mut value = fixture("mcq", "valid-enzyme-kinetics.json");
    value["verification_status"] = "admitted".into();
    value["contract_version"] = serde_json::Value::Null;
    let record: McqRecord = serde_json::from_value(value).unwrap();
    assert_eq!(
        record.validate(),
        Err(vec![ValidationError::AdmittedWithoutContractVersion])
    );

    let mut value = fixture("mcq", "valid-enzyme-kinetics.json");
    value["verification_status"] = "admitted".into();
    value["contract_version"] = "0.1".into();
    let record: McqRecord = serde_json::from_value(value).unwrap();
    assert!(record.validate().is_ok());
}

#[test]
fn model_requires_prompt_version() {
    let mut record = mcq();
    record.provenance.model = Some("teacher-model-v2".into());
    assert_eq!(
        record.validate(),
        Err(vec![ValidationError::ModelWithoutPromptVersion])
    );

    record.provenance.prompt_version = Some("mcq-draft-prompt-v3".into());
    assert!(record.validate().is_ok());
}

#[test]
fn subdomain_must_match_domain() {
    let mut record = mcq();
    record.domain = medsynth_schema::Domain::Cpf; // subdomain stays macromolecules (bbf)
    assert_eq!(
        record.validate(),
        Err(vec![ValidationError::DomainSubdomainMismatch {
            domain: "cpf".into(),
            subdomain: "macromolecules".into(),
        }])
    );
}

#[test]
fn key_must_match_an_option() {
    let mut record = mcq();
    record.key = medsynth_schema::OptionLabel::E; // fixture has A-D
    assert_eq!(
        record.validate(),
        Err(vec![ValidationError::KeyMatchesNoOption {
            key: "E".into()
        }])
    );
}

#[test]
fn schema_version_matches_schema_numeric_semantics() {
    // The generated schema is `{"type": "integer", "const": 1}`; JSON Schema
    // treats 1.0 as the integer 1, so both spellings must load.
    assert!(serde_json::from_value::<SchemaVersion>(serde_json::json!(1)).is_ok());
    assert!(serde_json::from_value::<SchemaVersion>(serde_json::json!(1.0)).is_ok());
    assert!(serde_json::from_value::<SchemaVersion>(serde_json::json!(1.5)).is_err());
    assert!(serde_json::from_value::<SchemaVersion>(serde_json::json!(2)).is_err());
    assert!(serde_json::from_value::<SchemaVersion>(serde_json::json!("1")).is_err());
    // Exact decimal comparison: a version that merely *rounds* to 1.0 must
    // not load and be rewritten as v1.
    assert!(serde_json::from_str::<SchemaVersion>("1.0000000000000001").is_err());
    assert!(serde_json::from_str::<SchemaVersion>("1e0").is_ok());
}
