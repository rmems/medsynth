# Dataset records

Admitted dataset records live here, one directory per format — matching the
taxonomy §3.3 format slugs:

- `mcq/` — single-best-answer multiple-choice questions
- `case_vignette/` — clinical-style vignettes with an associated question
- `tutoring_example/` — worked instructional examples with explicit reasoning

Records are JSONL, one record per line, conforming to the generated schemas
in `schemas/` (`<format>.v<N>.json`). Only records that have passed the
verification pipeline (#9) and been admitted by the admission gate are
written here — `unverified` drafts never enter `data/`.

Generation and admission are handled by the pipeline crates (#8, #9); this
directory is the published artifact, not a staging area.

## Gold seed staging exception (#7)

`gold/<format>.jsonl` holds agent-authored reference seeds pending pipeline
admission, always at `unverified`. The separate agent review evidence is in
`gold/review-manifest.json`; it does not advance verification status.
Only these three explicit shards are candidate few-shot material. Records
under `gold/near-miss/` are deliberately defective verification probes,
never examples for generation and never published dataset records.
Neither seeds nor probes belong in the admitted format directories above.
