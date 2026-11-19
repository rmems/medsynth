# medsynth Content Contract

**Status:** binding for all records, generators, verifiers, and reviewers in this
repository.
**Version:** 1.0 — first issued contract; see §9 for versioning rules.
**Operationalized by:** record schemas (#4), anchor source registry (#5),
curriculum taxonomy (#3), verification pipeline (#9) — each lands via its own
issue; §8 describes how they enforce this contract.

This document is the law under which medsynth operates. Every generated,
verified, audited, and published record is judged against it. Where a pipeline
stage, schema field, or review checklist appears to conflict with this contract,
the contract wins — fix the artifact, not the rule.

## 1. Scope

medsynth produces synthetic training and evaluation data for pre-medical and
early medical education: multiple-choice questions (MCQs), case vignettes, and
tutoring examples. The dataset exists to **teach and test knowledge**. It is not
a clinical product, not a diagnostic tool, and not medical advice.

These rules apply to every record at every stage — drafted, `unverified`,
`verified`, `admitted`, or published — and to every artifact derived from them
(datasheets, eval outputs, documentation examples).

## 2. Fictional patients only

- Every patient, clinician, institution, and encounter in a record is invented.
- No real cases, no protected health information (PHI), and no identifiable
  details from real clinical encounters — no real names, dates, locations,
  record numbers, or biographical details that could single out a person.
- Presentations, laboratory values, imaging descriptions, and histories are
  constructed. Where realism requires specifics (age ranges, vitals, lab
  panels), they are invented to be *plausible*, never copied from a case report,
  textbook patient, or published vignette.
- De-identified case reports and published vignettes are still **sources of
  facts, not of patients**: a record may anchor its clinical content to them,
  but its patient must not reproduce them.

## 3. Original items only

- Every item is written for medsynth. No reproduced, paraphrased, or
  "inspired-by" MCAT, USMLE, COMLEX, or commercial question-bank items.
- Board-*style* format is expected and encouraged (single-best-answer stems,
  vignette format, rationales). Board *content* is prohibited — that means
  reproduced, paraphrased, or derived exam items, not subject-matter overlap.
  An original item may test knowledge that also appears on board exams; that
  is the point of the dataset.
- Stems are built from the curriculum taxonomy (#3) and anchored facts — never
  retrieved from or templated off existing question banks.
- The originality & contamination audit (#11) is the enforcement arm of this
  rule; a record that fails it does not ship regardless of other scores.

## 4. Source anchoring

**Every clinical or scientific fact in a record carries a source anchor.** A
fact that cannot be anchored does not ship.

- An *anchor* cites an approved source from the anchor source registry (#5) —
  a standard textbook, guideline, or recognized reference — plus a locator
  (chapter + section, or page where the registry's convention provides one).
- Anchors reference registry source IDs only. A fact whose only support is an
  unlisted source is unanchored until that source is approved in the registry.
- Every factual claim in stems, answer rationales, teaching points, and
  explanations is either anchored or is pure pedagogy (definitions of terms,
  instructions to the learner) that asserts no clinical fact.
- Anchors must support the claim they are attached to — citing a general
  reference for a specific numeric claim (dose, threshold, timeline) does not
  count. Resolution is necessary but not sufficient: the verification pipeline
  (#9) must check that the cited passage actually supports the claim, not
  merely that the locator resolves.
- **Invented patient observations are exempt from anchoring.** Vitals, lab
  values, histories, and imaging findings invented for a fictional patient
  (§2) are observations, not claims — they must be plausible, not cited. The
  exemption covers the observation only: any claim *about* it ("this troponin
  pattern indicates...", "these vitals are consistent with...") is a clinical
  fact and must be anchored.
- **Distractors anchor to their refutation.** An MCQ distractor is a wrong
  answer by design; no source can be asked to support it as written, and it
  may even be a *true* statement that is not the best answer in context.
  Distractors are anchored through the rationale's statement of *why* the
  option is wrong — the anchor supports that refutation (false claim, or
  true-but-not-best-here), not the option's wording.
- Prose that *implies* facts ("the drug is contraindicated in pregnancy")
  carries the same anchoring duty as explicit claims.

## 5. Educational framing

- Records teach and test knowledge. They are not medical advice and must never
  be written as diagnosis or treatment instructions addressed to a real person.
- Vignettes describe fictional patients for the learner to reason about; they
  do not instruct the reader to act on a real patient's behalf.
- Rationales explain *why* an answer is correct in educational terms
  (mechanism, guideline concordance, classic teaching point) rather than
  issuing directives.
- Records for tutoring examples may walk through reasoning explicitly, but the
  framing remains instructional: "consider," "the typical presentation is,"
  "first-line management per [anchored guideline]" — not "you should prescribe."

## 6. Prohibited content

A record containing any of the following is rejected outright:

- Real-person diagnosis or treatment recommendations, or content that could be
  mistaken for care of an identifiable individual.
- Unanchored dosing or management claims presented as authoritative.
- Deprecated guidelines, withdrawn recommendations, or superseded standards
  presented as current practice.
- Reproduced or near-reproduced commercial exam content (§3).
- Any real patient data or PHI (§2).
- Content outside the curriculum taxonomy's scope unless the taxonomy is
  amended first (#3).

## 7. Currency rule

- Every anchor to a guideline, recommendation, or time-sensitive reference
  **names the edition or year**. Staleness must be visible at a glance —
  "ACC/AHA 2022" not "ACC/AHA."
- Anchors to living documents (web resources, continuously updated databases)
  cite the access date in addition to the source's own version identifier.
- When a new edition of an anchored guideline publishes, affected records are
  flagged for re-verification. Withdrawals, retractions, safety
  communications, and errata trigger the same recheck even without a new
  edition — a recommendation pulled in place still makes dependent records
  stale. Superseded sources remain in the registry marked
  as such so historical anchors stay interpretable (see #5).
- A record anchored only to superseded sources may remain in the dataset only
  if it is explicitly labeled historical/superseded in its metadata; otherwise
  it is re-anchored or rejected.

## 8. Enforcement

This contract is enforced mechanically and procedurally:

- **Schemas (#4)** encode the anchor fields, provenance fields, and status enum
  this contract requires. A record that cannot express its anchors is not
  schema-valid.
- **Verification pipeline (#9)** is the correctness gate: schema validation,
  answer-key re-derivation, source-anchor resolution *and* claim-support
  checks (the cited passage must actually support the claim, not merely
  resolve), distractor plausibility, and the human review sample.
  `unverified` drafts never reach the dataset.
- **Transient vs. invalid anchors.** A registry or resolver outage is an
  infrastructure failure, not a missing anchor: affected records are held and
  retried, and the admission log reports infrastructure failures separately
  from genuine anchor defects so outages don't masquerade as rejections.
- **Originality audit (#11)** screens for contamination and near-duplicates.
- **CI data validation (#10)** runs schema and anchor checks on every change.
- **Admission log** records, per batch, how many records were admitted and
  rejected at each stage — the contract's bite is measured there.
- **Gold seed set (#7)** defines the reference standard; deviations from it in
  generated output are defects to be explained, not precedents to follow.

## 9. Contract versioning

- This contract is versioned. Changes that tighten or clarify rules take effect
  for new records immediately; already-admitted records are grandfathered unless
  a change is safety-critical, in which case affected records are re-verified.
- The admission gate verifies against the currently effective contract version
  and rejects or holds any record processed by a stale verifier — a record may
  never enter the dataset under rules that are no longer in force.
- Every admitted record stores the contract version it was admitted under, so
  the dataset can always answer "which rules produced this record?"
