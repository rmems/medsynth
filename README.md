# medsynth

[![License: Apache 2.0 OR MIT](https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue.svg)](LICENSE-APACHE)
[![Status](https://img.shields.io/badge/status-v0.1%20in%20construction-orange.svg)](https://github.com/rmems/medsynth/issues)

Synthetic training and evaluation data for pre-medical and early medical
education — every fact traceable to a reference.

medsynth produces three record formats: multiple-choice questions (MCQs),
clinical-style case vignettes, and tutoring examples with explicit reasoning.
The v0.1 target is **300 admitted, verified records** across the curriculum
taxonomy, published under a datasheet with full provenance.

## Correctness-first

Medical data lives or dies on factual correctness, so medsynth is built
correctness-first: **no record ships on generator confidence alone.** Every
record must pass a verification pipeline (schema validation, answer-key
re-derivation, source-anchor resolution *and* claim-support checks) before
it counts toward the dataset. The full rules are binding — see
[`docs/contract.md`](docs/contract.md).

Contract highlights:

- **Fictional patients only** — no real cases, no PHI, no identifiable
  details from real clinical encounters.
- **Original items only** — board-*style* format, never board *content*; no
  reproduced or paraphrased MCAT/USMLE/commercial items.
- **Source anchoring** — every clinical or scientific fact carries a source
  anchor to an approved reference plus a locator. A fact that cannot be
  anchored does not ship.
- **Educational framing** — records teach and test knowledge; they are not
  medical advice.
- **Currency** — guideline anchors name edition/year so staleness is visible.

## Record formats

| Format | What it is |
|---|---|
| `mcq` | Single-best-answer multiple-choice question with rationales |
| `case_vignette` | Clinical-style vignette with an associated question |
| `tutoring_example` | Worked instructional example with explicit reasoning |

Each format has three difficulty tiers — `foundation`, `application`,
`reasoning` — defined per format in the taxonomy. Domains: the three MCAT
domains (`bbf`, `cpf`, `psb`) plus `pre_clinical` systems.

## Repository layout

```
crates/            Rust workspace — record types, validation, pipelines
  medsynth-schema/ Record types (serde + schemars), JSON Schema generation
schemas/           Generated JSON Schemas (do not hand-edit; see schemas/README.md)
fixtures/          Valid/invalid fixture records for data-validation CI
data/              Dataset records, one directory per format
docs/              contract.md (binding), taxonomy.md
scripts/           Generation + verification entry points (planned)
```

`data/` is laid out by format (`data/mcq/`, `data/case_vignette/`,
`data/tutoring_example/`); records are JSONL conforming to the schemas in
`schemas/`.

## Status

**v0.1 in construction.** Landed so far: content contract (#2), curriculum
taxonomy + coverage matrix (#3), record schemas (#4). In flight: gold seed
set, generation pipeline, verification & admission pipeline, CI data
validation, originality audit, eval harness, dataset assembly, and Hugging
Face publication. See the [issue tracker](https://github.com/rmems/medsynth/issues)
for the live plan.

## Development

Rust workspace, pinned toolchain (`rust-toolchain.toml`, edition 2024).

```sh
cargo test -p medsynth-schema   # schema + fixture + taxonomy tests
cargo run -p medsynth-schema --bin schema-gen   # regenerate schemas/
```

See [CONTRIBUTING.md](CONTRIBUTING.md) — the content contract is binding on
every contribution.

## License

Dual-licensed under **Apache-2.0 OR MIT** — use whichever you prefer. See
[LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT). Anyone may
contribute or use the dataset.
