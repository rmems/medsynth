# medsynth-generate

Candidate-record generation for medsynth. This crate builds the two lanes from
issue #8 and proves them on the committed sample batch. It does **not** generate
the 300-record v0.1 pool, and it never admits a record: every draft stays
`verification_status: "unverified"` until the verification & admission pipeline
(#9) acts on it.

## Lanes

### Procedural

Deterministic templates for templatable MCQs:

- **Fact recall** — `fact.*` entries in `src/procedural/facts.rs`. The stem and
  the correct option come from one anchored fact; the key is the label the
  correct answer landed on after the seeded shuffle. The rationale's refutation
  carries the anchor for the distractors.
- **Calculation** — `calc.*` entries in `src/procedural/calc.rs`. The template
  samples parameters, writes those same parameters into the stem, computes the
  answer from them, and only then shuffles the options. The key is therefore
  derived, never asserted.

Each record's PCG stream is derived from the batch seed plus the cell and
template identity, so the same seed and config reproduce the batch byte for
byte. Every procedural record sets `provenance.template_version`.

### Teacher

Pinned model drafting for case vignettes, tutoring examples, and
non-templatable MCQs. Gold records from `data/gold/<format>.jsonl` are supplied
as few-shot *style* exemplars; the prompt requires the teacher to build only
from the supplied taxonomy cell and approved anchor points. Teachers return
content-only JSON. The pipeline stamps the record id, taxonomy, versions, and
provenance; a teacher never sets verification status. For MCQs the teacher
returns a `proposed_key`, which remains a proposal and is re-derived by #9.

The teacher lane is **fail-closed**:

1. It reads `docs/teachers.md` before constructing a client or dispatching a
   prompt.
2. The selected pin must exist, be complete, be classified `teacher`, belong to
   the DeepSeek or Nemotron 3 family, and affirm both `permits_distillation`
   and `permits_synthetic_data`. Missing or research-only pins (Claude, Grok)
   refuse the lane.
3. The teacher lane never calls a teacher API itself. It writes each
   request to the `--teacher-responses` fulfillment directory as
   `<key>.request.json` — an OpenAI-compatible chat-completion body — and
   reads the response the external harness drops at `<key>.response.json`.
   The key is a hash of the exact request body, so fulfillments cannot be
   mixed up. A missing response drops the attempt; when every attempt is
   unfulfilled the lane reports an error, and the procedural lane still
   writes its output.
4. Credentials live only in the external harness, under the env var named
   in the pin entry (`DEEPSEEK_API_KEY`, `NVIDIA_API_KEY` — informational
   fields). The repo holds, reads, and logs no API keys.

Prompts and completions are budgeted with `tiktoken-rs` (cl100k_base as the
budgeting tokenizer). The system prompt forbids reproducing or paraphrasing
known exam items, requires fictional patients, and restricts citations to the
supplied source id / locator pairs. A teacher draft whose anchor is not in the
cell's approved list is dropped by self-validation.

## CLI

Run from the workspace root:

```sh
cargo run -p medsynth-generate -- templates
cargo run -p medsynth-generate -- matrix
cargo run -p medsynth-generate -- check-pin
cargo run -p medsynth-generate -- run \
  --config drafts/sample-batch/config.json \
  --out drafts/sample-batch
```

`run` accepts `--seed` to override the config seed, `--lane
all|procedural|teacher` to restrict the run, and `--teacher-responses <dir>`
to point at the fulfillment directory shared with the external harness
(required when any cell uses the teacher lane). Defaults are
`docs/taxonomy.md`, `docs/sources.json`, `docs/teachers.md`, `schemas/`, and
`data/gold/`.

The output directory **must not be under `data/`**. Admitted records belong in
`data/<format>/` only after #9 admission; drafts must never enter `data/gold/`.

## Config

```json
{
  "seed": 2523,
  "cells": [
    {
      "domain": "bbf",
      "format": "mcq",
      "tier": "foundation",
      "count": 2,
      "lane": "procedural",
      "template": "fact.alveolar_epithelium",
      "subdomain": "animal_structure",
      "resolved_attempts": 0,
      "in_flight_attempts": 0
    }
  ],
  "teacher": {
    "pin_id": "deepseek-v4.1-flash"
  }
}
```

- Every target cell must exist in the taxonomy §5 matrix. `count` is a draft
  request, not admitted fill: it may exceed the matrix target to budget
  replacement drafts, but never the 3× draft cap. The run clamps dispatch to
  the remaining budget and records `budget_limited`.
- `resolved_attempts` and `in_flight_attempts` are the attempts already
  resolved or in flight for that cell outside this batch. The run never
  dispatches drafts that would push
  `resolved_attempts + in_flight_attempts + dispatched` past `3 × matrix
  target` (taxonomy §5.2). A cell at the cap dispatches nothing and is marked
  `escalated`.
- Procedural cells require a template id and are MCQ-only. Teacher cells use
  the optional `teacher` selection, and `prompt_version`, token budgets,
  exemplar count, and temperature have defaults.
- The subdomain must belong to the domain (taxonomy §3.2).

## Manifest

`manifest.json` is written beside the JSONL shards. It carries:

- `seed`, the effective `config`, and `config_sha256`;
- hashes/versions of `docs/taxonomy.md`, `docs/sources.json`, and
  `docs/teachers.md`;
- the teacher pin registry summary, the selected `teacher_pin` reference, and
  the teacher-lane status/error;
- per-cell `matrix_target`, `requested`, `resolved_attempts`,
  `in_flight_attempts`, `budget_cap`, `dispatched`, `produced`, `dropped`,
  `budget_limited`, and `escalated`;
- per-record SHA-256 of the exact compact JSON line, plus the JSONL filename;
- `dropped_by_self_validation` and `dropped_by_generation`, with per-draft
  reasons and stages;
- `rights_envelopes` for teacher drafts: record id, teacher variant, exact
  pin entry, its hash, and the record content hash;
- `anchor_receipts` for every anchor: record id, source id, locator, the
  SHA-256 of the exact registry entry bytes, and the registry version.

The record types have no hash, seed, envelope, or receipt fields; those live in
the manifest.

## Self-validation

Before writing, every draft is:

1. validated against `schemas/<format>.v1.json`;
2. deserialized into its `medsynth-schema` type and passed through
   `validate()`;
3. checked for lane-appropriate provenance, `unverified` status, the `0.1`
   taxonomy/contract versions, registry-resolved anchors, and (for teacher
   drafts) the cell's anchor allowlist.

A failing draft is dropped and counted; it is never written. Teacher drafts
also need a complete rights envelope; an incomplete envelope drops the draft
and counts it under `dropped_by_self_validation`.

## Tests

```sh
cargo test -p medsynth-generate
```

The suite covers procedural byte reproducibility, independent recomputation of
each sampled calculation key, fact-key derivation, provenance/receipt
completeness, the 3× budget, the safe output directory, pin refusal
(unpinned/incomplete/research-only/terms), teacher-lane refusal without a
fulfillment directory while the procedural lane still runs, recorded-client
request/response file round-trips, rights-envelope drops, and
teacher-response parsing against the recorded fixtures under
`tests/fixtures/teacher/`. No test makes a live API call.

## Dependencies

The crate uses the workspace's existing stack. `rand_core 0.6` is added as a
direct dependency because `rand_pcg 0.3` implements that crate's `RngCore` and
`SeedableRng` traits; the traits must be in scope to drive the seeded PCG
stream. No other new dependency is introduced, and the crate carries no HTTP
client — teacher fulfillment is file-based.
