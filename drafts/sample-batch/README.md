# Sample generation batch

A small reviewable batch produced by `medsynth-generate` from the procedural
lane. Every record is `unverified`; none is admitted and nothing here belongs
in `data/`.

## What is here

- `config.json` — seed `2523` and six target cells: `bbf` fact recall (1),
  `cpf` SI-unit fact recall (1) plus a Newton's-second-law calculation cell
  (2), `psb` fact recall (1), and `pre_clinical` fact recall (1) plus a
  half-life calculation cell (2).
- `mcq.jsonl` — the eight generated MCQ drafts.
- `manifest.json` — seed, effective config, input-document hashes, per-cell
  counts, per-record SHA-256 hashes, anchor receipts, and drop counts. The
  batch has zero self-validation drops and zero generation drops.

Regenerate byte for byte from the workspace root:

```sh
cargo run -p medsynth-generate -- run \
  --config drafts/sample-batch/config.json \
  --out drafts/sample-batch
```

The same seed and config reproduce the same bytes; `cargo test -p
medsynth-generate` asserts that.

## Why there are no vignette or tutoring shards

The teacher lane was not run for this batch because no recorded teacher
responses existed yet. The pin (`docs/teachers.md`) is in place and the gate
is tested, but the issue says teacher drafts are included only when real
teacher output is available; inventing teacher output would be worse than
leaving the format absent.

The teacher lane's prompt, response parsing, rights-envelope, allowlist, and
drop behavior are exercised by the recorded fixtures under
`crates/medsynth-generate/tests/fixtures/teacher/` with no live API calls. To
add real teacher drafts, run a teacher config with a fulfillment directory;
the pipeline writes `<key>.request.json` files there, an external harness
dispatches them to the pinned endpoint (it owns the API key), and writes the
raw responses back as `<key>.response.json`. For a Merge-routed pin, preserve
routing metadata too: either the body contains `routing.vendor_used` (or an
equivalent vendor field), or the harness copies a header-only `x-merge-vendor`
into `<key>.vendor.txt` as `<sha256(response-json)> <vendor>`:

```sh
cargo run -p medsynth-generate -- run \
  --config path/to/teacher-config.json \
  --out drafts/teacher-batch \
  --teacher-responses path/to/fulfillments
```

A teacher cell looks like this (omit `template`; the lane is chosen by `lane`):

```json
{
  "seed": 2524,
  "cells": [
    {
      "domain": "pre_clinical",
      "format": "case_vignette",
      "tier": "application",
      "count": 2,
      "lane": "teacher",
      "subdomain": "respiratory"
    },
    {
      "domain": "bbf",
      "format": "tutoring_example",
      "tier": "foundation",
      "count": 2,
      "lane": "teacher",
      "subdomain": "animal_structure"
    }
  ],
  "teacher": { "pin_id": "deepseek-v4.1-flash" }
}
```

## Reading the manifest

- `records[].sha256` hashes the exact compact JSON line in the shard.
- `anchor_receipts[]` pins each anchor to the exact `docs/sources.json` entry
  bytes used for this batch, so #9 can verify a receipt rather than trust a
  bare locator.
- `rights_envelopes` is empty here because there are no teacher drafts. When
  present, each envelope carries the record id, teacher variant, exact
  `docs/teachers.md` pin entry, and the draft's content hash.
- `teacher_lane.status` is `not_requested`; `teacher_pin` is null.

Drafts belong in an output directory outside `data/`; `medsynth-generate`
refuses to write under `data/` at all.
