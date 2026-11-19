//! Regenerate the checked-in JSON Schemas in `schemas/` from the record
//! types. Run after changing any type in this crate:
//!
//! ```sh
//! cargo run -p medsynth-schema --bin schema-gen
//! ```

use std::path::Path;

fn main() -> std::io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the workspace root")
        .join("schemas");
    std::fs::create_dir_all(&root)?;

    for file in medsynth_schema::schema_files() {
        // Write to a temp file and rename so an interrupted run never leaves a
        // truncated schema on disk.
        let path = root.join(&file.filename);
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, file.to_json_string())?;
        std::fs::rename(&tmp, &path)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
