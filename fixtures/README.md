# Fixture records

Fixture records for the data-validation CI in #10, one directory per format
(`mcq/`, `case_vignette/`, `tutoring_example/` — the taxonomy §3.3 format
slugs).

- `valid-*.json` must pass schema validation, deserialize into the record
  type, and pass cross-field `validate()`.
- `invalid-*.json` must fail schema validation on **exactly one rule** — the
  planted defect is in the filename — so a red fixture is diagnosable at a
  glance.

The crate's fixture tests (`cargo test -p medsynth-schema`) enforce both
directions.

**Source ids are placeholders.** Anchor `source_id` values like
`example-biochemistry-9e` are pattern-valid stand-ins until the anchor source
registry (#5) publishes real ids. Swapping them for registry ids is part of
wiring #5 into #4/#9, and only matters once validation resolves anchors
against the registry rather than just checking the id pattern.
