# Near-miss verification probes

These six **deliberately defective, unverified** records are not gold exemplars,
few-shot material, or admitted dataset records. The `data/gold/` staging
exception does not authorize publishing them in the format directories.
Each has exactly one planted semantic defect. `manifest.json` is the
machine-readable companion to this table; independent confirmation belongs
in `../review-manifest.json`.

| File / record id | Planted defect | Required rejection |
| --- | --- | --- |
| `mcq/wrong-key-0001.json` / `near-miss-mcq-wrong-key-0001` | Key B instead of A (exocrine); the explanation and refutations remain correct. | #9 stage 2, blind key re-derivation |
| `mcq/wrong-key-0002.json` / `near-miss-mcq-wrong-key-0002` | Key A instead of C (social stratification); explanation unchanged. | #9 stage 2, blind key re-derivation |
| `mcq/missing-anchor-0001.json` / `near-miss-mcq-missing-anchor-0001` | The factual Newton's-law statement in `stem.text` has `stem.anchors: []`; rationale anchors do not supply the absent stem anchor. | #9 stage 3, claim-level anchor check |
| `mcq/missing-anchor-0002.json` / `near-miss-mcq-missing-anchor-0002` | The formal-norm definition in the stem has no attached anchor. | #9 stage 3, claim-level anchor check |
| `tutoring_example/unanchored-claim-0001.json` / `near-miss-tut-unanchored-claim-0001` | The response's epithelial claims are anchored, but the separate ADH synthesis claim is not supported by the cited epithelial section. | #9 stage 3, passage-support check |
| `tutoring_example/unanchored-claim-0002.json` / `near-miss-tut-unanchored-claim-0002` | The response's wave claims are anchored, but the appended hydrostatic-pressure claim has no supporting passage in the cited wave section. | #9 stage 3, passage-support check |

All six must pass JSON Schema, typed deserialization, and cross-field
`validate()`. For the missing-anchor probes we use `AnchoredText.anchors: []`,
which the schema permits for claim-free text, but which is defective here
because the stems assert facts. Removing the required `anchors` property,
or emptying an `AnchoredClaim` array, would fail at parsing/schema validation
and would not isolate the intended stage-3 defect.

The #10 CI `key-rationale` smoke check catches the two wrong keys. It does
**not** implement #9's semantic re-derivation or claim-support checks:
the remaining four intentionally pass its shallow checks at `unverified`.
Their rejection stage is a verification expectation confirmed by the
independent review, not a claim that #9 already exists. No stage-4 defect
is planted in this set.
