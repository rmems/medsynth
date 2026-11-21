# Gold seed set (#7, RM-2522)

Agent-authored reference seeds, drafted by `factory-droid-draft-rm2522-v1`
under checklist `gold-review-v1` (recorded in each record's
`provenance.generator`; `model`, `prompt_version`, and `template_version` are
null). Every record stays at `verification_status: "unverified"`; only the #9
pipeline may advance it. These shards are staging, not admitted data.

- `mcq.jsonl`, `case_vignette.jsonl`, `tutoring_example.jsonl` — 24 seeds,
  2 per format × domain cell, all three tiers per format. These three shards
  are the only candidate few-shot material.
- `near-miss/` — 6 deliberately defective probes plus their manifest
  (`near-miss/README.md`). Never few-shot material, never admitted.
- `review-manifest.json` — evidence from a separate reviewing agent run
  (a different session from the drafting run): blind key re-derivation,
  passage-level anchor support, rationale and distractor checks, tier check,
  and planted-defect confirmation. Each entry pins the reviewed record by
  SHA-256, so `cargo test -p medsynth-schema` fails if a record changes after
  review.

## Sources

All anchors cite `current` entries in `docs/sources.json`, using each
source's locator convention: `openstax-anatomy-physiology-2e`,
`openstax-university-physics-2016`, and `openstax-introduction-sociology-3e`.
They were chosen because their section text is openly readable, so the
reviewer can check that each cited passage supports its claim instead of
only checking that the locator resolves. Concentrating on three sources is a
known gap: later seeds should broaden to the clinical references in the
registry.

## Review record

Key re-derivation: the reviewer solved every MCQ and vignette from a blind
copy of the stems, options, findings, and questions before opening the keys
or rationales. Its answer and comparison are in `review-manifest.json`
(`key_check.blind_answer`, `key_check.evidence`). Tutoring examples have no
key (`not_applicable`).

| Record | Domain / subdomain | Tier | Key | Anchors | Tier rationale (taxonomy §4) |
| --- | --- | --- | --- | --- | --- |
| gold-mcq-bbf-0001 | bbf / animal_structure | foundation | A | A&P `sec4.2:epithelial-tissue` | Recall of one junction function; no scenario. |
| gold-mcq-bbf-0002 | bbf / system_interactions | application | B | A&P `sec17.3`, `sec26.2` | One inference from a short osmolarity scenario to the ADH response. |
| gold-mcq-cpf-0001 | cpf / mechanics | application | C | Physics `vol1:ch5:newtons-second-law` | Apply F = ma to a brief self-contained scenario. |
| gold-mcq-cpf-0002 | cpf / transmission | reasoning | D | Physics `vol1:ch16` wave speed + mathematics of waves | Chains two relations (v from tension, then λ = v/f); each distractor breaks one link. |
| gold-mcq-psb-0001 | psb / culture_identity | foundation | B | Sociology `ch3:elements-of-culture` | Recognize one defined term. |
| gold-mcq-psb-0002 | psb / socioeconomic | application | A | Sociology `ch9:what-is-social-stratification` | Apply the wealth/income distinction to survey measures. |
| gold-mcq-preclinical-0001 | pre_clinical / respiratory | application | C | A&P `sec22.4:gas-exchange` | One inference: thickness change to diffusion rate. |
| gold-mcq-preclinical-0002 | pre_clinical / endocrine | reasoning | D | A&P `sec17.3`, `sec26.2` | Integrates baseline pattern and hormone response to discriminate competing mechanisms. |
| gold-cv-bbf-0001 | bbf / animal_structure | foundation | — | A&P `sec4.2` | Short vignette, one identification by pattern-matching. |
| gold-cv-bbf-0002 | bbf / system_interactions | reasoning | — | A&P `sec26.2`, `sec17.3` | Integrates time course across hypothalamic, endocrine, and renal steps; rejects a competing explanation. |
| gold-cv-cpf-0001 | cpf / mechanics | application | — | Physics `vol1:ch5` | Connect presented forces to the principle and compute. |
| gold-cv-cpf-0002 | cpf / properties_of_matter | foundation | — | Physics `vol1:ch14:fluids-density-and-pressure` | One identification: pressure rises with depth. |
| gold-cv-psb-0001 | psb / culture_identity | application | — | Sociology `ch3` | Connect observed findings to the symbol concept. |
| gold-cv-psb-0002 | psb / socioeconomic | reasoning | — | Sociology `ch9`, `ch19` | Weighs competing explanations across a sequence and states remaining uncertainty. |
| gold-cv-preclinical-0001 | pre_clinical / gastrointestinal | foundation | — | A&P `sec23.7` | Single anchored teaching point on bile-salt function. |
| gold-cv-preclinical-0002 | pre_clinical / respiratory | reasoning | — | A&P `sec22.5`, `sec22.4` | Reconciles a normal PO2 with reduced carriage; distinguishes PO2, saturation, and content. |
| gold-tut-bbf-0001 | bbf / animal_structure | foundation | — | A&P `sec4.2` | Walks through one distinction; no problem posed. |
| gold-tut-bbf-0002 | bbf / system_interactions | application | — | A&P `sec26.2`, `sec17.3` | Step-by-step through a self-contained causal chain, naming each inference. |
| gold-tut-cpf-0001 | cpf / properties_of_matter | application | — | Physics `vol1:ch14` fluids + measuring pressure | Guided solution of a typical hydrostatic problem. |
| gold-tut-cpf-0002 | cpf / transmission | reasoning | — | Physics `vol3:ch2:images-formed-by-refraction` | Analyzes a plausible wrong turn and the approximation's limits. |
| gold-tut-psb-0001 | psb / culture_identity | foundation | — | Sociology `ch3` | One concept set with a canonical example. |
| gold-tut-psb-0002 | psb / environment | reasoning | — | Sociology `ch20`, `ch9` | Compares individual and structural hypotheses and names where reasoning overreaches. |
| gold-tut-preclinical-0001 | pre_clinical / endocrine | foundation | — | A&P `sec17.3` | Single mechanism sketch: synthesis vs. release site. |
| gold-tut-preclinical-0002 | pre_clinical / cardiovascular | reasoning | — | A&P `sec20.2` | Corrects a wrong turn (linear vs. r⁴) and states model limits. |

## Gaps

- No records were dropped. Coverage is the minimum 2 per cell; subdomains
  without a seed (for example `macromolecules`, `translation`, `development`,
  `renal`, `neurology`) remain open for later seeds.
- The anchors draw on three open textbooks; clinical guideline anchors
  (contract §7 currency) are not yet represented in the gold set.
