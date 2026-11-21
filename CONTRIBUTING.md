# Contributing to medsynth

Thanks for your interest. medsynth is a synthetic dataset for pre-medical and
early medical education, and its value depends entirely on factual
correctness — so contributions are held to a high bar.

**Read [`docs/contract.md`](docs/contract.md) first.** It is binding on every
record, generator, verifier, and reviewer in this repository. The short
version:

- Fictional patients only — no real cases, no PHI.
- Original items only — board-style format, never board content.
- Every clinical or scientific fact carries a source anchor to an approved
  reference; unanchored facts do not ship. (Invented patient observations —
  vitals, labs, histories, imaging findings — are fictional and
  plausibility-checked, not anchored; contract §4.)
- Records teach and test knowledge — they are not medical advice.
- Guideline anchors name edition/year.

## How to contribute

AI-assisted contributions are welcome — feel free to use coding agents, and
feel free to open issues for anything you'd like to see (records, tooling,
docs, pipeline improvements). The contract applies to agent-written code
exactly as it does to human-written code.

1. Open an issue describing the change before writing code for anything
   non-trivial.
2. Branch from `main`; keep commits focused.
3. Rust workspace, edition 2024, toolchain pinned in
   `rust-toolchain.toml`. Shared dependencies are declared in the workspace
   `Cargo.toml`; member crates opt in via `workspace = true`.
4. `cargo test -p medsynth-schema` (and the relevant crate's tests) must pass.
   If you change record types, regenerate schemas with
   `cargo run -p medsynth-schema --bin schema-gen` — the drift test will
   fail otherwise.
5. Never hand-edit files under `schemas/` or `data/`; they are generated or
   admission-gated.

## License

By contributing you agree your contributions are licensed under **both**
Apache-2.0 **and** MIT — downstream users choose which license to take.
