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

## Citation-support audit (#26)

`medsynth-citation-audit` is the deeper citation check: it resolves the text
at each cited locator where `docs/citation-access.json` says the source
license permits text access, submits claim/passage pairs to an independent
adjudicator, and writes a per-run report containing record id, claim path,
anchor, verdict, passage SHA-256, passage URL, access basis, and adjudicator
run id. Passage text itself is confined to the working cache under `target/`
and is never written to `data/` or emitted in the report.

```sh
cargo run -p medsynth-validate --bin medsynth-citation-audit -- \
  --root . \
  --records data/gold \
  --gold-review data/gold/review-manifest.json \
  --report reports/citation-audit/gold.json
```

Sources absent from `docs/citation-access.json` are not skipped: every anchor
to one is reported as `unverifiable` with access `receipt-level-only`, and the
source is named in `unverifiable_by_design_sources`. Verdicts are
`supported`, `unsupported`, or `unverifiable`. `unsupported` is
release-blocking: the binary exits nonzero and the report's
`release_blocking_record_ids` is the exclusion list release assembly must
apply until the record is re-anchored and re-audited.

Semantic adjudication uses the same recorded-fulfillment pattern as admission:
the audit writes `<key>.audit.request.json` files containing claim text,
passage text, and passage hash, then reads hash-bound
`<key>.audit.response.json` files with a verdict and non-empty independent
`run_id`. A response missing, malformed, unbound, or using an
unknown/drafting run id is `unverifiable`, never silently supported. Response
`detail` text is not copied into the report, so reviewer prose cannot smuggle
passage text into the committed artifact. For the existing gold set only, the
independently produced `data/gold/review-manifest.json` is accepted as the
adjudication when its record SHA-256 matches exactly and quoted review
evidence still appears in the retrieved passage; a hash-bound response for
the current passage takes precedence when present. Future batches use the
request/response exchange.

The committed gold report is `reports/citation-audit/gold.json`. It was
generated from the three independent CC BY-SA 4.0 LibreTexts sources in the
access policy: 24 records, 103 claim blocks, 108 anchor occurrences, all
supported, across 24 distinct retrieved passages. The policy records license
provenance and exact allowlisted retrieval URLs; passage text stays out of the
committed report.

## Originality & contamination audit (#11)

`medsynth-originality-audit` is the release-candidate audit behind the smoke
screen above. It compares item prompts — MCQ stems, vignette presentations,
and tutoring learner prompts — against a rights-documented public reference
corpus, detects same-cell internal duplication, and groups same-anchor-passage
near-duplicates so dataset assembly can keep them split-aware.

```sh
cargo run -p medsynth-validate --bin medsynth-originality-audit -- \
  --root . \
  --records data/gold \
  --records drafts/sample-batch/mcq.jsonl \
  --references docs/contamination-reference.jsonl \
  --report reports/originality-audit/candidate.json
```

The audit is deterministic and writes JSON containing record ids, coverage
cells, prompt/record hashes, anchor-passage keys, pair counts, containment
scores, deterministic flag ids, contamination groups, split-check status,
dispositions, and the release-blocking record-id exclusion list. It never
copies external reference text into the report.

Three comparisons are distinct:

- **External overlap:** prompt 5-gram containment against every entry in the
  reference corpus; the default review threshold is 0.6.
- **Internal dedup:** same domain/format/tier cell plus overlapping anchor
  passages plus prompt containment at the default 0.35 threshold.
- **Contamination hygiene:** same-anchor-passage near-duplicates are unioned
  into groups. When `--splits` is supplied, one group assigned to multiple
  splits is a release-blocking cross-split violation.

Any flag starts as `needs-review` and blocks release. A `--dispositions` JSON
file binds human review to the deterministic `flag_id` with a decision of
`rewrite`, `replace`, or `not-a-duplicate`, plus an independent
`reviewer_run_id` and rationale. `rewrite`/`replace` remain release-blocking
until the candidate data changes; `not-a-duplicate` clears only a reviewed
false positive. Unused disposition ids are reported, never silently ignored.

`docs/contamination-reference.md` documents the JSONL reference-entry schema
and provenance requirements. The seed corpus is intentionally small and
rights-documented; release sign-off requires expanding it across the public
sample-item landscape before the 300-record v0.1 audit can be called complete.
