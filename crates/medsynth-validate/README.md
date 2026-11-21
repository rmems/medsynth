# medsynth-validate

The repo's data-validation gate — one Rust binary, no Python, no shell
scripts. Run from the workspace root:

```sh
cargo run -p medsynth-validate -- .
```

It validates every `*.json` record and `*.jsonl` shard under `data/`
(including the three flat `data/gold/<format>.jsonl` shards)
and every `valid-*.json` fixture under `fixtures/`, reports every failure
with the file, record id, check name, and the failing rule/path, and exits
nonzero if any record fails. The intentionally defective
`data/gold/near-miss/` subtree and `data/gold/review-manifest.json` metadata
are excluded from the positive corpus. Unknown data files are not excluded.

## Checks

| Check | What it enforces |
| --- | --- |
| `schema` | JSON Schema validation against the generated schemas from `medsynth-schema` (the single source of truth — this crate duplicates no schema rules), then deserialization into the typed record and its cross-field `validate()`. |
| `key-rationale` | Rationale present and non-empty and referencing the keyed answer: for `mcq`, `key_explanation`/`strongest_distractor` text is non-blank and anchored, and the keyed option shares more content words (lowercased tokens, len ≥ 4) with `key_explanation` than any other option — a wrong key leaves the rationale explaining a different option. For `case_vignette`, every `questions[].answer` and `teaching_points[]` claim is non-blank and anchored; for `tutoring_example`, `tutor_response.text` is non-blank. |
| `anchor-presence` | No record with `verification_status` above `unverified` (`key-checked`, `anchor-checked`, `admitted` — the declaration order of `medsynth-schema`'s `VerificationStatus`) may carry zero anchors anywhere in the record. This gate never advances a status. |
| `originality` | Smoke originality screen (below). |

## Smoke originality screen

A cheap, deterministic near-duplicate screen over record text fields —
the values of `text`, `presentation`, `question`, and all strings inside
`findings`/`options` arrays, lowercased and split on non-alphanumerics.

- **Shingles:** word **5-grams**, hashed into a set per record.
- **Threshold:** a record is flagged when its **containment** in another
  record's shingle set — `|shared 5-grams| / |its 5-grams|` — reaches
  **0.6**. Both records of a pair are reported.

A copied record shares ~1.0 containment; two records merely on the same
topic share few 5-grams. This is the smoke screen only — the full
originality & contamination audit is #11.

## Gold and near-miss probes (#7)

`data/gold/near-miss/manifest.json` identifies each planted defect and its
required #9 rejection stage. These probes are never loaded as positive
corpus records. The fixture-behavior integration test loads them explicitly:
all must pass schema/deserialization/cross-field checks, and the two wrong
keys must fail only `key-rationale`. The four claim-level omissions
intentionally pass the shallow smoke gate at `unverified`; #9 stage 3 must
reject them by examining attached anchors and actual passage support.
The `ci_check` field is null for these semantic defects, not a promised
rejection by a nonexistent verifier.

`cargo test -p medsynth-schema` checks the gold/near-miss record shapes,
coverage, registry locator syntax, and per-record independent review
evidence, bound to the exact record content by SHA-256. Neither suite
advances a verification status or substitutes for #9 admission.
