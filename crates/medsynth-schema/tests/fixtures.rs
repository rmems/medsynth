//! Fixture records for the CI in #10: every `valid-*.json` under `fixtures/`
//! must validate against its format's generated schema (and deserialize +
//! pass cross-field `validate`), and every `invalid-*.json` must fail —
//! each on exactly one rule, so a red fixture is diagnosable at a glance.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use medsynth_schema::{
    CaseVignetteRecord, McqRecord, TutoringExampleRecord, ValidationError, schema_files,
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .to_path_buf()
}

fn schemas() -> HashMap<String, jsonschema::Validator> {
    schema_files()
        .into_iter()
        .map(|f| {
            (
                f.filename,
                jsonschema::validator_for(&f.schema).expect("generated schema compiles"),
            )
        })
        .collect()
}

fn fixture_files(format_dir: &str, prefix: &str) -> Vec<PathBuf> {
    let dir = workspace_root().join("fixtures").join(format_dir);
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(prefix) && n.ends_with(".json"))
        })
        .collect();
    files.sort();
    files
}

/// Lets the test call each record type's cross-field `validate` generically.
trait HasValidate {
    fn validate(&self) -> Result<(), Vec<ValidationError>>;
}
impl HasValidate for McqRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        McqRecord::validate(self)
    }
}
impl HasValidate for CaseVignetteRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        CaseVignetteRecord::validate(self)
    }
}
impl HasValidate for TutoringExampleRecord {
    fn validate(&self) -> Result<(), Vec<ValidationError>> {
        TutoringExampleRecord::validate(self)
    }
}

fn read_instance(path: &Path) -> serde_json::Value {
    serde_json::from_str(
        &fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display())),
    )
    .unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

/// Assert a valid fixture passes schema validation, deserializes into the
/// record type, and passes cross-field `validate`.
fn assert_valid<T>(validator: &jsonschema::Validator, path: &Path)
where
    T: serde::de::DeserializeOwned + HasValidate,
{
    let instance = read_instance(path);

    let errors: Vec<_> = validator.iter_errors(&instance).collect();
    assert!(
        errors.is_empty(),
        "valid fixture {} failed schema validation:\n{}",
        path.display(),
        errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );

    let record: T = serde_json::from_value(instance).unwrap_or_else(|e| {
        panic!(
            "{} must deserialize into the record type: {e}",
            path.display()
        )
    });
    if let Err(errors) = record.validate() {
        panic!(
            "valid fixture {} failed cross-field validation: {errors:?}",
            path.display()
        );
    }
}

/// Assert an invalid fixture fails for exactly one reason — either one JSON
/// Schema rule or one cross-field `validate` rule.
fn assert_invalid<T>(validator: &jsonschema::Validator, path: &Path)
where
    T: serde::de::DeserializeOwned + HasValidate,
{
    let instance = read_instance(path);

    let errors: Vec<_> = validator.iter_errors(&instance).collect();
    if errors.is_empty() {
        // Schema-clean: the planted defect must live in the cross-field
        // layer (e.g. domain/subdomain pairing, key without an option).
        let record: T = serde_json::from_value(instance).unwrap_or_else(|e| {
            panic!(
                "{} must deserialize into the record type: {e}",
                path.display()
            )
        });
        let Err(errors) = record.validate() else {
            panic!(
                "invalid fixture {} passed both validation layers",
                path.display()
            );
        };
        assert_eq!(
            errors.len(),
            1,
            "invalid fixture {} must break exactly one rule, found {} cross-field errors: {errors:?}",
            path.display(),
            errors.len()
        );
    } else {
        assert_eq!(
            errors.len(),
            1,
            "invalid fixture {} must break exactly one rule, found {} schema errors:\n{}",
            path.display(),
            errors.len(),
            errors
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

fn check_format<T>(format_dir: &str, schema_name: &str)
where
    T: serde::de::DeserializeOwned + HasValidate,
{
    let schemas = schemas();
    let validator = &schemas[schema_name];
    for path in fixture_files(format_dir, "valid-") {
        assert_valid::<T>(validator, &path);
    }
    for path in fixture_files(format_dir, "invalid-") {
        assert_invalid::<T>(validator, &path);
    }
}

#[test]
fn mcq_fixtures() {
    check_format::<McqRecord>(
        "mcq",
        &format!("mcq.v{}.json", medsynth_schema::CURRENT_SCHEMA_VERSION),
    );
}

#[test]
fn case_vignette_fixtures() {
    check_format::<CaseVignetteRecord>(
        "case_vignette",
        &format!(
            "case_vignette.v{}.json",
            medsynth_schema::CURRENT_SCHEMA_VERSION
        ),
    );
}

#[test]
fn tutoring_example_fixtures() {
    check_format::<TutoringExampleRecord>(
        "tutoring_example",
        &format!(
            "tutoring_example.v{}.json",
            medsynth_schema::CURRENT_SCHEMA_VERSION
        ),
    );
}
