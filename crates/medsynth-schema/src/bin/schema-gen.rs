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
        let path = root.join(&file.filename);
        std::fs::write(&path, file.to_json_string())?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
