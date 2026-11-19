//! The JSON Schemas in `schemas/` are generated from the record types; this
//! test fails when the checked-in files no longer match what the types
//! generate — run `cargo run -p medsynth-schema --bin schema-gen` to fix.

use std::path::Path;

fn schemas_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .join("schemas")
}

#[test]
fn checked_in_schemas_match_generated() {
    for file in medsynth_schema::schema_files() {
        let path = schemas_dir().join(&file.filename);
        let checked_in = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "cannot read {}: {e} — run `cargo run -p medsynth-schema --bin schema-gen`",
                path.display()
            )
        });
        assert_eq!(
            checked_in,
            file.to_json_string(),
            "{} is stale — regenerate with `cargo run -p medsynth-schema --bin schema-gen`",
            file.filename
        );
    }
}
