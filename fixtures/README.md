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

`citation-audit/` holds schema-valid negative fixtures for #26. They are not
`valid-*.json` corpus fixtures because their planted defect is semantic rather
than structural: `negative-unsupported-mcq.json` has a well-formed registry
citation whose key explanation asserts the wrong junction type, so the cited
OpenStax passage does not support it. The citation-audit integration test
fulfills it as `unsupported` through the independent-adjudicator exchange and
checks that the verdict carries the passage hash.

**Source ids are real registry ids.** Anchor `source_id` values cite the
approved sources in `docs/sources.md` / `docs/sources.json` (#5); the
`source_registry` test in `medsynth-schema` proves every id cited by the
valid fixtures exists in the registry and that every registry id matches
`SOURCE_ID_PATTERN`. Only `valid-*.json` fixtures are checked against the
registry — `invalid-*.json` fixtures carry deliberately broken fields (e.g.
`tutoring_example/invalid-bad-source-id.json`).
