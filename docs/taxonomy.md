# medsynth Curriculum Taxonomy

**Status:** binding for record schemas (#4), generation targets (#8), and the
assembly scoreboard (#13). The content contract (`docs/contract.md`) governs
what may be covered; this document defines *where* records may live. Content
outside the taxonomy is prohibited (contract §6) unless this document is
amended first.

**Version:** 0.1

## 1. Purpose

The taxonomy has three parts:

1. **Domain and subdomain value sets** — the enumerable vocabulary records
   classify under.
2. **Format and difficulty-tier definitions** — what a record *is* and what
   distinguishes one tier from another within each format.
3. **Coverage matrix** — target record counts per domain × format × tier,
   summing to the v0.1 target of **300 admitted, verified records**. The matrix is the
   generation target for #8 and the scoreboard for assembly in #13.

## 2. Scope

In scope for v0.1:

- **MCAT domains:** Biological & Biochemical Foundations; Chemical & Physical
  Foundations; Psychological, Social & Biological Foundations.
- **Pre-clinical systems:** cardiovascular, respiratory, renal,
  gastrointestinal, endocrine, neurology, musculoskeletal,
  hematology/oncology, infectious disease, pharmacology principles.

Out of scope for v0.1:

- **CARS (Critical Analysis and Reasoning Skills).** Passage licensing makes
  originality hard to guarantee under contract §3, so CARS content cannot be
  anchored to original passages yet. It is excluded entirely, not deferred to
  a subdomain.
- **Clinical clerkship / shelf-exam content** beyond the listed pre-clinical
  systems, graduate medical education, and board-review commercial content.

## 3. Value sets

The schemas in #4 lift the following enums verbatim. Do not add prose
variants, synonyms, or alternate spellings — add a new slug here first, then
use it.

### 3.1 `domain` — exactly 4 values

```yaml
domain:
  - bbf                 # Biological & Biochemical Foundations (MCAT)
  - cpf                 # Chemical & Physical Foundations (MCAT)
  - psb                 # Psychological, Social & Biological Foundations (MCAT)
  - pre_clinical        # Pre-clinical systems
```

Every record carries exactly one `domain`. `pre_clinical` is a first-class
domain alongside the three MCAT domains; its subdomains are the pre-clinical
systems listed in §2.

### 3.2 `subdomain` — exactly 25 values

Subdomains are unique slugs scoped under their parent domain. A record
carries one `domain` and one `subdomain`, and the pair must be consistent
with the table below.

```yaml
subdomain:
  # domain: bbf
  - bio_physical_foundations   # Chemical & physical foundations of biological systems (BB Foundations category 1)
  - macromolecules             # Structure and function of biological macromolecules
  - cell_function              # Principles of cell function
  - cell_processes             # Processes governing cellular organization and function
  - system_interactions        # Interactions between systems and organs
  - animal_structure           # Unique aspects of animal structure and function
  # domain: cpf
  - translation                # Translation of chemical and physical sciences
  - properties_of_matter       # Properties of matter
  - transmission               # Principles of transmission of information
  - mechanics                  # Principles of mechanics
  # domain: psb
  - development                # Development and lifespan learning processes
  - socioeconomic              # Socioeconomic status, education, and poverty
  - culture_identity           # Culture, language, and identity
  - psychology                 # Psychology and social behaviors
  - environment                # Environment and external factors
  # domain: pre_clinical
  - cardiovascular
  - respiratory
  - renal
  - gastrointestinal
  - endocrine
  - neurology
  - musculoskeletal
  - hematology_oncology
  - infectious_disease
  - pharmacology_principles
```

Parent mapping (for schema validation):

| Subdomain | Domain |
|---|---|
| `bio_physical_foundations`, `macromolecules`, `cell_function`, `cell_processes`, `system_interactions`, `animal_structure` | `bbf` |
| `translation`, `properties_of_matter`, `transmission`, `mechanics` | `cpf` |
| `development`, `socioeconomic`, `culture_identity`, `psychology`, `environment` | `psb` |
| `cardiovascular`, `respiratory`, `renal`, `gastrointestinal`, `endocrine`, `neurology`, `musculoskeletal`, `hematology_oncology`, `infectious_disease`, `pharmacology_principles` | `pre_clinical` |

### 3.3 `format` — exactly 3 values

```yaml
format:
  - mcq                # Single-best-answer multiple-choice question
  - case_vignette      # Clinical-style vignette with an associated question
  - tutoring_example   # Worked instructional example with explicit reasoning
```

### 3.4 `tier` — exactly 3 values

```yaml
tier:
  - foundation
  - application
  - reasoning
```

Tier is defined **per format**. The same tier name means different things in
an MCQ, a case vignette, and a tutoring example — a record's tier is always
interpreted relative to its `format`.

## 4. Difficulty tier definitions

Difficulty measures cognitive demand, not topical hardness. All three tiers
are held to the same contract standards (§4 anchoring, §5 framing): a
foundation record is not allowed to be sloppier than a reasoning one.

### 4.1 `mcq`

| Tier | Definition |
|---|---|
| `foundation` | Tests recall or direct comprehension of a single anchored fact or concept. The stem supplies everything needed; answering requires one step — retrieve or recognize — with no inference from a scenario. |
| `application` | Requires applying a mechanism, principle, or relationship to a brief, self-contained scenario or novel phrasing. One inference step separates the stem from the key; distractors are plausible single-concept confusions. |
| `reasoning` | Requires integrating two or more concepts, discriminating between competing mechanisms or diagnoses, or weighing evidence across a multi-part stem. The key is defensible only through chained reasoning; distractors each exploit a distinct plausible error in that chain. |

### 4.2 `case_vignette`

| Tier | Definition |
|---|---|
| `foundation` | A short vignette presenting an uncomplicated fictional patient; the question asks for a basic identification — the likely mechanism, the expected finding, or the single best first concept — resolvable by pattern-matching to one anchored teaching point. |
| `application` | The vignette requires connecting presented findings (history, vitals, labs) to the underlying mechanism or principle and selecting the interpretation or next conceptual step. Findings map cleanly onto one system or concept. |
| `reasoning` | The vignette requires integrating findings across systems or time course, prioritizing among competing interpretations, or reconciling a finding that fits more than one explanation. The learner must justify why one interpretation outranks the alternatives. |

### 4.3 `tutoring_example`

| Tier | Definition |
|---|---|
| `foundation` | Walks the learner through a single concept or fact: definition, mechanism sketch, or canonical example, with the reasoning made explicit at each step. No novel problem is posed. |
| `application` | Guides the learner step-by-step through solving a typical, self-contained problem or clinical reasoning task, naming each inference as it is made. |
| `reasoning` | Models multi-step reasoning on a non-trivial problem: comparing competing hypotheses, analyzing a plausible wrong turn, or extending a principle to an edge case — including where the reasoning could go wrong and why. |

## 5. Coverage matrix

Target counts of **admitted, verified** records for v0.1, by `domain` ×
`format` × `tier`. The matrix sums to exactly **300**. A cell's target is met
only by records that have passed verification *and* admission — `unverified`
drafts do not count toward fill.

Cell notation below is `F/A/R` = foundation / application / reasoning.

| Domain | Format | Foundation | Application | Reasoning | Format subtotal |
|---|---|---|---|---|---|
| `bbf` | `mcq` | 18 | 16 | 8 | 42 |
| `bbf` | `case_vignette` | 6 | 7 | 5 | 18 |
| `bbf` | `tutoring_example` | 4 | 4 | 4 | 12 |
| **`bbf` total** | | **28** | **27** | **17** | **72** |
| `cpf` | `mcq` | 12 | 11 | 7 | 30 |
| `cpf` | `case_vignette` | 4 | 4 | 4 | 12 |
| `cpf` | `tutoring_example` | 2 | 2 | 2 | 6 |
| **`cpf` total** | | **18** | **17** | **13** | **48** |
| `psb` | `mcq` | 10 | 9 | 5 | 24 |
| `psb` | `case_vignette` | 4 | 4 | 4 | 12 |
| `psb` | `tutoring_example` | 2 | 2 | 2 | 6 |
| **`psb` total** | | **16** | **15** | **11** | **42** |
| `pre_clinical` | `mcq` | 32 | 32 | 20 | 84 |
| `pre_clinical` | `case_vignette` | 12 | 13 | 11 | 36 |
| `pre_clinical` | `tutoring_example` | 6 | 6 | 6 | 18 |
| **`pre_clinical` total** | | **50** | **51** | **37** | **138** |
| **Grand total** | | **112** | **110** | **78** | **300** |

### 5.1 Arithmetic check

Format totals (sum across domains):

- `mcq`: 42 + 30 + 24 + 84 = **180**
- `case_vignette`: 18 + 12 + 12 + 36 = **78**
- `tutoring_example`: 12 + 6 + 6 + 18 = **42**
- Format total: 180 + 78 + 42 = **300** ✓

Domain totals (sum across formats):

- `bbf`: 42 + 18 + 12 = **72**
- `cpf`: 30 + 12 + 6 = **48**
- `psb`: 24 + 12 + 6 = **42**
- `pre_clinical`: 84 + 36 + 18 = **138**
- Domain total: 72 + 48 + 42 + 138 = **300** ✓

Tier totals (sum across all cells):

- `foundation`: 28 + 18 + 16 + 50 = **112**
- `application`: 27 + 17 + 15 + 51 = **110**
- `reasoning`: 17 + 13 + 11 + 37 = **78**
- Tier total: 112 + 110 + 78 = **300** ✓

Distribution rationale: `pre_clinical` carries the largest share (138, 46%)
because the v0.1 dataset serves pre-medical and early medical learners where
systems-level clinical reasoning is the core need. MCQs dominate (180, 60%)
as the highest-throughput verifiable format; case vignettes (78, 26%) and
tutoring examples (42, 14%) round out applied and instructional coverage.
Foundation and application tiers are weighted above reasoning (~37% / 37% /
26%) so the dataset teaches before it tests.

### 5.2 Usage

- **Generation (#8):** produces records against per-cell targets and stops at
  each exact cell target — a cell at target is complete, under-target cells
  define remaining work. Generators do not over-produce: 300 is the v0.1
  total, not a floor. The cap applies to *admitted fill*, not draft count:
  records rejected at verification or audit may be replaced by new drafts
  until the cell's admitted count reaches its target.
- **Assembly (#13):** the admitted, verified dataset is tallied against this
  matrix; the scoreboard reports per-cell fill and the grand total against
  the 300-record v0.1 target. Generation completion and assembly scoring use
  the same status (admitted + verified) so the two can never disagree.
- Subdomain-level distribution *within* a domain × format × tier cell is a
  generation concern, not a matrix constraint — the matrix counts at domain
  granularity only. Generators should still spread coverage across a
  domain's subdomains rather than clustering on a few.

## 6. Amendment rules

- Adding a domain, subdomain, format, or tier requires amending this document
  first; schema enums (#4) then update to match.
- This document is versioned alongside the content contract. Records store
  the taxonomy version they were classified under (in addition to the
  contract version required by contract §9), so validators can select the
  enum set that was in force when the record was admitted — the same
  grandfathering mechanism contract §9 applies to contract versions.
- The admission gate applies the same staleness rule to taxonomy versions as
  contract §9 applies to contract versions: a record processed under a stale
  taxonomy version is rejected or held, never admitted. A draft may outlive
  its classification version; it is re-classified under the current taxonomy
  or held before admission.
- Changing a coverage target requires the new matrix to re-reconcile to the
  stated total (300 for v0.1) with the arithmetic shown.
- Removing a slug is a breaking change: records already admitted under it
  must be migrated or the taxonomy version pinned per contract §9.
