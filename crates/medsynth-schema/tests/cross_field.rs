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
    // numeric equality accepts 1.0, 1e0, 10e-1, 0.1e1 — so must loading.
    for token in [
        "1", "1.0", "1e0", "1e+0", "1E0", "10e-1", "0.1e1", "1.00e0", "100e-2",
    ] {
        assert!(
            serde_json::from_str::<SchemaVersion>(token).is_ok(),
            "{token} equals 1 and must load"
        );
    }
    for token in ["1.5", "2", "0.9", "20e-1", "\"1\"", "true", "null", "[1]"] {
        assert!(
            serde_json::from_str::<SchemaVersion>(token).is_err(),
            "{token} must not load as v1"
        );
    }
    // Exact decimal comparison: a version that merely *rounds* to 1.0 must
    // not load and be rewritten as v1.
    assert!(serde_json::from_str::<SchemaVersion>("1.0000000000000001").is_err());
    // serde_json's arbitrary-precision object marker must not substitute for
    // a real number.
    assert!(
        serde_json::from_str::<SchemaVersion>(r#"{"$serde_json::private::Number": "1"}"#).is_err()
    );
    // from_value goes through the same check (arbitrary_precision preserves
    // the raw token inside Value).
    assert!(serde_json::from_value::<SchemaVersion>(serde_json::json!(1)).is_ok());
    assert!(serde_json::from_value::<SchemaVersion>(serde_json::json!(1.0)).is_ok());
    assert!(serde_json::from_value::<SchemaVersion>(serde_json::json!(1.5)).is_err());
    assert!(serde_json::from_value::<SchemaVersion>(serde_json::json!("1")).is_err());
}

#[test]
fn schema_gate_version_is_f64_numeric() {
    // The jsonschema crate compares `const` numerically as f64: exact-1
    // spellings pass the schema gate, and so does a decimal that merely
    // rounds to 1 — the Rust loader is the stricter second layer for those.
    let schema = medsynth_schema::schema_files()
        .into_iter()
        .find(|f| f.filename.starts_with("mcq"))
        .expect("mcq schema")
        .schema;
    let validator = jsonschema::validator_for(&schema).unwrap();
    let base = fixture("mcq", "valid-enzyme-kinetics.json");
    for token in ["1", "1.0", "1e0", "10e-1", "0.1e1", "1.0000000000000001"] {
        let mut v = base.clone();
        v["schema_version"] = serde_json::from_str(token).unwrap();
        assert!(validator.is_valid(&v), "{token} passes the schema gate");
    }
    let mut v = base.clone();
    v["schema_version"] = serde_json::from_str("1.5").unwrap();
    assert!(!validator.is_valid(&v));
    // …but the loader still rejects the rounding spell.
    let mut v = base.clone();
    v["schema_version"] = serde_json::from_str("1.0000000000000001").unwrap();
    assert!(serde_json::from_value::<McqRecord>(v).is_err());
}

#[test]
fn string_enums_reject_object_variant_form() {
    // serde's derived enum impl accepts the externally-tagged object form
    // ({"A": null} -> A); our enums deserialize from strings only, matching
    // the generated schema's string `enum`.
    assert!(serde_json::from_str::<medsynth_schema::OptionLabel>("\"A\"").is_ok());
    assert!(serde_json::from_str::<medsynth_schema::OptionLabel>("{\"A\":null}").is_err());
    assert!(serde_json::from_str::<medsynth_schema::Domain>("\"bbf\"").is_ok());
    assert!(serde_json::from_str::<medsynth_schema::Domain>("{\"Bbf\":null}").is_err());
    assert!(serde_json::from_str::<medsynth_schema::VerificationStatus>("\"admitted\"").is_ok());
    assert!(
        serde_json::from_str::<medsynth_schema::VerificationStatus>("{\"Admitted\":null}").is_err()
    );
    assert!(serde_json::from_str::<medsynth_schema::Tier>("{\"Foundation\":null}").is_err());
    assert!(serde_json::from_str::<medsynth_schema::TutoringStyle>("{\"Socratic\":null}").is_err());
    assert!(
        serde_json::from_str::<medsynth_schema::RecordLabel>("{\"HistoricalSuperseded\":null}")
            .is_err()
    );
    assert!(serde_json::from_str::<medsynth_schema::Subdomain>("{\"Renal\":null}").is_err());
}
