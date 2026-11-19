# Scripts

Thin entry points for the generation and verification pipelines.

The pipeline logic lives in the Rust workspace (`crates/`) — these scripts
are conveniences for humans and CI, not a second implementation.

Planned:

- `generate/` — batch record generation (#8): procedural templates +
  teacher-model drafting, seeded for reproducibility.
- `verify/` — the correctness gate (#9): schema validation, answer-key
  re-derivation, anchor resolution and claim-support checks, admission.

Until those crates land, this directory holds only this note. Anything a
script does must enforce `docs/contract.md` — the contract is binding on
scripts exactly as it is on the code they wrap.
