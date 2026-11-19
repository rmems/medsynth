//! Generation of the checked-in JSON Schemas under `schemas/`.
//!
//! The record types are the source of truth; these files are build output.
//! The schema-gen binary writes them and the drift test asserts the checked-in
//! copies still match — regenerate with:
//!
//! ```sh
//! cargo run -p medsynth-schema --bin schema-gen
//! ```

use crate::records::{CaseVignetteRecord, McqRecord, TutoringExampleRecord};
use crate::version::CURRENT_SCHEMA_VERSION;

/// One generated schema file: its filename under `schemas/` and its contents.
pub struct SchemaFile {
    pub filename: String,
    pub schema: serde_json::Value,
}

impl SchemaFile {
    /// Pretty-printed JSON, ending in a newline — the exact bytes written to
    /// `schemas/` and compared by the drift test.
    pub fn to_json_string(&self) -> String {
        let mut s = serde_json::to_string_pretty(&self.schema).expect("schema serializes");
        s.push('\n');
        s
    }
}

/// The generated JSON Schema for each record format, filename included.
///
/// Each filename and `$id` carries the schema version (`v1`, …): bump
/// [`CURRENT_SCHEMA_VERSION`] and add the new filenames here when the record
/// shape changes incompatibly.
pub fn schema_files() -> Vec<SchemaFile> {
    [
        ("mcq", schemars::schema_for!(McqRecord)),
        ("case_vignette", schemars::schema_for!(CaseVignetteRecord)),
        (
            "tutoring_example",
            schemars::schema_for!(TutoringExampleRecord),
        ),
    ]
    .into_iter()
    .map(|(format, schema)| {
        let filename = format!("{format}.v{CURRENT_SCHEMA_VERSION}.json");
        let mut value = serde_json::to_value(&schema).expect("schema serializes");
        value.as_object_mut().expect("schema is an object").insert(
            "$id".into(),
            serde_json::Value::String(format!(
                "https://raw.githubusercontent.com/rmems/medsynth/main/schemas/{filename}"
            )),
        );
        SchemaFile {
            filename,
            schema: value,
        }
    })
    .collect()
}
