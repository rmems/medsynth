# Contamination reference corpus

`docs/contamination-reference.jsonl` is the seed reference corpus for the
originality & contamination audit (#11). Each line is one public exercise or
sample item with this shape:

```json
{
  "id": "stable-reference-id",
  "kind": "public-exercise",
  "text": "exact reference text used for overlap comparison",
  "source_url": "canonical public URL",
  "license": "license identifier",
  "provenance": "attribution and exact excerpt provenance"
}
```

The audit hashes this corpus into its report and compares candidate item
prompts against the text, but the report never copies reference text. Entries
must be exact excerpts or clearly bounded excerpts; whitespace may be
normalized to single spaces, but wording must not be paraphrased because
paraphrase would make overlap evidence unauditable.

The seed entries are exercises from the CC BY-SA 4.0 LibreTexts textbook
*Introductory Physics: Building Models to Describe Our World*. They are a
small machine-checkable seed, not a claim of exhaustive public-item coverage.
Before a release candidate is signed off, expand this corpus with additional
rights-documented public sample items across MCAT-style biology, chemistry,
physics, psychology/sociology, and pre-clinical domains. The license field is machine-checked. The current allowlist is `CC0-1.0`,
`CC BY 4.0`, `CC BY-SA 4.0`, and `public-domain`; expand it only after a
rights review. Entries whose license or ingestion terms are incompatible —
including OpenStax CC BY-NC-SA pages with AI-ingestion restrictions — fail
closed and must not be added.

When `--splits` is supplied, every audited record id must have an assignment.
Incomplete assignments fail closed because the cross-split check cannot be
sound otherwise.
