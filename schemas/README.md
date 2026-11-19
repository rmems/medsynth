# Generated JSON Schemas

These files are **generated** from the Rust record types in
`crates/medsynth-schema` with schemars — the types are the source of truth.

Do **not** hand-edit anything in this directory. After changing the record
types, regenerate with:

```sh
cargo run -p medsynth-schema --bin schema-gen
```

A drift test (`cargo test -p medsynth-schema`) fails if the checked-in files
fall out of sync with the types.

Files are versioned in the filename and `$id` (`<format>.v<N>.json`), matching
the `schema_version` field every record carries.
