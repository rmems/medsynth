# Anchor Source Registry

**Status:** binding for record schemas (#4), the verification pipeline (#9),
and CI data validation (#10). `docs/sources.json` is the machine-readable copy
of this registry — the two files must stay **entry-for-entry identical** (same
source ids, same fields). If they ever disagree, that is a defect, not a
follow-up.

Governing rules:

- `docs/contract.md` §4 (source anchoring): every clinical or scientific fact
  carries an anchor citing a registry source id plus a locator; unlisted
  sources are unanchored.
- `docs/contract.md` §7 (currency rule): edition/year are mandatory and never
  blank; living documents carry an access date; superseded editions stay
  listed and marked `superseded`, never deleted.
- `docs/taxonomy.md` §3.2: coverage uses the exact taxonomy subdomain slugs —
  no prose variants.

## How anchors cite a source

An anchor is `{ "source_id": "...", "locator": "..." }` where:

- `source_id` matches `SOURCE_ID_PATTERN` in
  `crates/medsynth-schema/src/anchor.rs` — a lowercase slug, 3–64 characters,
  edition-scoped so the edition/year is visible in the id itself
  (e.g. `campbell-biology-13e`, `ada-standards-diabetes-2025`). Ids are never
  reused or re-spelled once published.
- `locator` is written in the locator convention published for that source —
  typically `ch<N>:<section-slug>` for textbooks or `sec<N>` / `rec<N>` for
  guidelines — so #9 can resolve it mechanically. Anchors to living-document
  entries (those carrying an access date, e.g. `ada-standards-diabetes-2025`)
  must carry the access date of the anchor check in the locator per contract
  §7.
- Living guideline entries are re-checked when a new edition appears: the new
  edition is added as a `current` entry, the old one flips to `superseded`,
  and the coverage table is updated — never silently overwritten.

Adding a source is a reviewed change: new entries must be verifiable against
the publisher or issuing body and name who added them and why the source is
authoritative for its claimed domains. A subdomain with no approved source
cannot be generated for until the registry is amended.

## Coverage matrix

Every taxonomy §3.2 subdomain and the registry source ids approved to anchor
it. No row is empty: every subdomain in v0.1 scope has at least one approved
source.

| Subdomain | Domain | Source ids |
|---|---|---|
| `bio_physical_foundations` | `bbf` | `campbell-biology-13e`, `campbell-biology-12e` |
| `macromolecules` | `bbf` | `lippincott-biochem-9e`, `mol-bio-cell-6e` |
| `cell_function` | `bbf` | `mol-bio-cell-6e` |
| `cell_processes` | `bbf` | `campbell-biology-13e`, `campbell-biology-12e`, `mol-bio-cell-6e` |
| `system_interactions` | `bbf` | `campbell-biology-13e`, `campbell-biology-12e`, `mol-bio-cell-6e`, `openstax-anatomy-physiology-2e` |
| `animal_structure` | `bbf` | `campbell-biology-13e`, `campbell-biology-12e`, `openstax-anatomy-physiology-2e` |
| `translation` | `cpf` | `chemistry-central-science-15e` |
| `properties_of_matter` | `cpf` | `chemistry-central-science-15e`, `openstax-university-physics-2016` |
| `transmission` | `cpf` | `openstax-university-physics-2016` |
| `mechanics` | `cpf` | `openstax-university-physics-2016` |
| `development` | `psb` | `myers-psychology-14e` |
| `socioeconomic` | `psb` | `openstax-introduction-sociology-3e` |
| `culture_identity` | `psb` | `openstax-introduction-sociology-3e` |
| `psychology` | `psb` | `myers-psychology-14e` |
| `environment` | `psb` | `myers-psychology-14e`, `openstax-introduction-sociology-3e` |
| `cardiovascular` | `pre_clinical` | `openstax-anatomy-physiology-2e`, `guyton-hall-physiology-14e`, `harrison-principles-22e`, `harrison-principles-21e`, `braunwald-heart-disease-12e`, `acc-aha-af-guideline-2023`, `robbins-kumar-basic-pathology-11e` |
| `respiratory` | `pre_clinical` | `openstax-anatomy-physiology-2e`, `guyton-hall-physiology-14e`, `harrison-principles-22e`, `harrison-principles-21e`, `robbins-kumar-basic-pathology-11e` |
| `renal` | `pre_clinical` | `openstax-anatomy-physiology-2e`, `guyton-hall-physiology-14e`, `harrison-principles-22e`, `harrison-principles-21e`, `robbins-kumar-basic-pathology-11e` |
| `gastrointestinal` | `pre_clinical` | `openstax-anatomy-physiology-2e`, `guyton-hall-physiology-14e`, `harrison-principles-22e`, `harrison-principles-21e`, `robbins-kumar-basic-pathology-11e` |
| `endocrine` | `pre_clinical` | `openstax-anatomy-physiology-2e`, `guyton-hall-physiology-14e`, `harrison-principles-22e`, `harrison-principles-21e`, `ada-standards-diabetes-2025`, `robbins-kumar-basic-pathology-11e` |
| `neurology` | `pre_clinical` | `openstax-anatomy-physiology-2e`, `harrison-principles-22e`, `harrison-principles-21e`, `adams-victor-neurology-12e`, `robbins-kumar-basic-pathology-11e` |
| `musculoskeletal` | `pre_clinical` | `openstax-anatomy-physiology-2e`, `harrison-principles-22e`, `harrison-principles-21e`, `firestein-kelley-rheumatology-12e`, `robbins-kumar-basic-pathology-11e` |
| `hematology_oncology` | `pre_clinical` | `harrison-principles-22e`, `harrison-principles-21e`, `williams-hematology-10e`, `robbins-kumar-basic-pathology-11e` |
| `infectious_disease` | `pre_clinical` | `harrison-principles-22e`, `harrison-principles-21e`, `robbins-kumar-basic-pathology-11e`, `sherris-medical-microbiology-7e`, `idsa-iai-update-2024` |
| `pharmacology_principles` | `pre_clinical` | `harrison-principles-22e`, `harrison-principles-21e`, `katzung-basic-clinical-pharmacology-16e` |

## Registry entries

### `campbell-biology-13e`

- **Bibliographic identity:** *Campbell Biology*, 13th ed. — Lisa A. Urry,
  Peter V. Minorsky, Kerry L. Hull, Rebecca B. Orr; Pearson; 2025.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `bio_physical_foundations`, `cell_processes`, `system_interactions`, `animal_structure`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section or topic, or a stable page number. Examples: `ch8:enzyme-kinetics`,
  `p.1247`.
- **Added by:** rmems
- **Rationale:** The standard introductory biology text; its chemistry-of-life,
  cell-process, and animal form-and-function units map directly onto the BB&F
  subdomains.

### `campbell-biology-12e`

- **Bibliographic identity:** *Campbell Biology*, 12th ed. — Lisa A. Urry,
  Michael L. Cain, Steven A. Wasserman, Peter V. Minorsky, Rebecca B. Orr;
  Pearson; 2021.
- **Kind:** textbook
- **Status:** superseded (superseded by `campbell-biology-13e`)
- **Coverage:** `bio_physical_foundations`, `cell_processes`, `system_interactions`, `animal_structure`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section or topic, or a stable page number. Examples: `ch8:enzyme-kinetics`,
  `p.1158`.
- **Added by:** rmems
- **Rationale:** Retained so historical anchors citing the 12th edition remain
  interpretable after the 13th edition superseded it (contract §7).

### `lippincott-biochem-9e`

- **Bibliographic identity:** *Lippincott Illustrated Reviews: Biochemistry*,
  9th ed. — Emine Ercikan Abali, Susan D. Cline, David S. Franklin, Susan M.
  Viselli; Wolters Kluwer Health (Lippincott); 2025.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `macromolecules`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents (enzyme kinetics and enzyme
  inhibition are in the early enzyme chapter), a lowercase hyphenated slug for
  the section, or a stable page number. Examples: `ch5:enzyme-kinetics`,
  `ch5:competitive-inhibition`.
- **Added by:** rmems
- **Rationale:** The standard medical-school biochemistry review; authoritative
  for protein/enzyme structure, function, and kinetics.

### `mol-bio-cell-6e`

- **Bibliographic identity:** *Molecular Biology of the Cell*, 6th ed. —
  Bruce Alberts, Rebecca Heald, Alexander Johnson, David Morgan, Martin Raff,
  Keith Roberts, Peter Walter; Garland Science; 2015.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `macromolecules`, `cell_function`, `cell_processes`, `system_interactions`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch4:protein-structure`,
  `ch12:cell-cycle-control`.
- **Added by:** rmems
- **Rationale:** The definitive molecular cell-biology text (Alberts);
  authoritative for macromolecule structure/function, cell physiology, and
  cell-cell and tissue interactions.

### `openstax-anatomy-physiology-2e`

- **Bibliographic identity:** *Anatomy and Physiology*, 2nd ed. — J. Gordon
  Betts, Peter Desaix, Eddie Johnson, Jody E. Johnson, Oksana Korol, Dean
  Kruse, Brandon Poe, James A. Wise, Mark Womble, Kelly A. Young; OpenStax
  (issuing body); OpenStax / Rice University; 2022.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `animal_structure`, `system_interactions`, `cardiovascular`, `respiratory`, `renal`, `gastrointestinal`, `endocrine`, `neurology`, `musculoskeletal`
- **Locator convention:** `ch<N>:<section-slug>` or `sec<N>.<M>:<section-slug>`
  — the book is organized into units of numbered chapters; cite the chapter
  number plus a section slug, or section number within the chapter. Examples:
  `ch19:heart-structure`, `ch20:coronary-circulation`.
- **Added by:** rmems
- **Rationale:** Widely adopted open anatomy and physiology text organized by
  body system; provides the structural and organ-system basis for
  animal_structure and the pre-clinical systems.

### `chemistry-central-science-15e`

- **Bibliographic identity:** *Chemistry: The Central Science*, 15th ed. —
  Theodore E. Brown, H. Eugene LeMay, Bruce E. Bursten, Catherine Murphy,
  Patrick Woodward, Matthew E. Stoltzfus; Pearson; 2023.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `translation`, `properties_of_matter`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch1:units-measurement`,
  `ch10:gas-laws`.
- **Added by:** rmems
- **Rationale:** The standard general-chemistry text; authoritative for
  matter, measurement, bonding, and the chemical principles MCAT
  chemical-physical foundations build on.

### `openstax-university-physics-2016`

- **Bibliographic identity:** *University Physics (Volumes 1–3)* — William
  Moebs, Samuel J. Ling, Jeff Sanny; OpenStax (issuing body); OpenStax /
  Rice University; Volumes 1–3; 2016.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `mechanics`, `transmission`, `properties_of_matter`
- **Locator convention:** `vol<N>:ch<N>:<section-slug>` — physics is split
  across three volumes; cite the volume number, then chapter number and
  section slug as in the print or web table of contents. Examples:
  `vol1:ch15:wave-speed`, `vol3:ch1:law-of-reflection`.
- **Added by:** rmems
- **Rationale:** Standard calculus-based physics reference; covers mechanics,
  oscillations, waves, electricity and magnetism, and optics — the
  transmission and mechanics foundations of the CPF domain.

### `myers-psychology-14e`

- **Bibliographic identity:** *Psychology*, 14th ed. — David G. Myers,
  C. Nathan DeWall, June Gruber; Macmillan Learning (Worth Publishers); 2024.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `development`, `psychology`, `environment`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch6:operant-conditioning`,
  `ch11:social-psychology`.
- **Added by:** rmems
- **Rationale:** The most widely adopted introductory psychology text; covers
  lifespan development, learning and social behavior, and environmental
  influences on behavior.

### `openstax-introduction-sociology-3e`

- **Bibliographic identity:** *Introduction to Sociology*, 3rd ed. — Tonja R.
  Conerly, Kathleen Holmes, Asha Lal Tamang; OpenStax (issuing body);
  OpenStax / Rice University; 2021.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `socioeconomic`, `culture_identity`, `environment`
- **Locator convention:** `ch<N>:<section-slug>` — chapter number from the
  table of contents plus a lowercase hyphenated slug for the section.
  Examples: `ch8:social-stratification`, `ch12:gender-sexuality`.
- **Added by:** rmems
- **Rationale:** Standard open introductory sociology text; authoritative for
  social stratification, education and poverty, culture, language, and
  identity.

### `guyton-hall-physiology-14e`

- **Bibliographic identity:** *Guyton and Hall Textbook of Medical
  Physiology*, 14th ed. — John E. Hall, Michael E. Hall; Elsevier; 2020.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `cardiovascular`, `respiratory`, `renal`, `gastrointestinal`, `endocrine`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch21:coronary-circulation`,
  `ch69:gastric-secretion`.
- **Added by:** rmems
- **Rationale:** The standard medical physiology text; authoritative for the
  integrative physiology of the cardiovascular, respiratory, renal,
  gastrointestinal, and endocrine systems.

### `harrison-principles-22e`

- **Bibliographic identity:** *Harrison's Principles of Internal Medicine*,
  22nd ed. — Dan L. Longo, Anthony S. Fauci, Dennis L. Kasper, Stephen L.
  Hauser, J. Larry Jameson, Joseph Loscalzo, Steven M. Holland, Carol A.
  Langford (eds.); McGraw Hill; 2025.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `cardiovascular`, `respiratory`, `renal`, `gastrointestinal`, `endocrine`, `neurology`, `musculoskeletal`, `hematology_oncology`, `infectious_disease`, `pharmacology_principles`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch293:stemi`,
  `ch416:community-acquired-pneumonia`.
- **Added by:** rmems
- **Rationale:** The leading internal-medicine reference; system-by-system
  chapters give authoritative pathophysiology, diagnosis, and management
  coverage across all pre-clinical domains.

### `harrison-principles-21e`

- **Bibliographic identity:** *Harrison's Principles of Internal Medicine*,
  21st ed. — Joseph Loscalzo, Anthony S. Fauci, Dennis L. Kasper, Stephen L.
  Hauser, Dan L. Longo, J. Larry Jameson (eds.); McGraw Hill; 2022.
- **Kind:** textbook
- **Status:** superseded (superseded by `harrison-principles-22e`)
- **Coverage:** `cardiovascular`, `respiratory`, `renal`, `gastrointestinal`, `endocrine`, `neurology`, `musculoskeletal`, `hematology_oncology`, `infectious_disease`, `pharmacology_principles`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch290:stemi`, `p.2140`.
- **Added by:** rmems
- **Rationale:** Retained so historical anchors citing the 21st edition remain
  interpretable after the 22nd edition superseded it (contract §7).

### `braunwald-heart-disease-12e`

- **Bibliographic identity:** *Braunwald's Heart Disease: A Textbook of
  Cardiovascular Medicine*, 12th ed. — Peter Libby, Robert O. Bonow, Douglas
  L. Mann, Gordon F. Tomaselli, Deepak L. Bhatt, Scott D. Solomon, Eugene
  Braunwald (eds.); Elsevier; 2021.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `cardiovascular`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents (e.g. `ch14` is
  Electrocardiography), a lowercase hyphenated slug for the section, or a
  stable page number. Examples: `ch14:ecg-lead-groups`,
  `ch57:stemi-localization`.
- **Added by:** rmems
- **Rationale:** The definitive cardiovascular medicine text; authoritative for
  cardiac physiology, ECG interpretation, and ischemic heart disease.

### `acc-aha-af-guideline-2023`

- **Bibliographic identity:** *2023 ACC/AHA/ACCP/HRS Guideline for the
  Diagnosis and Management of Atrial Fibrillation* — American College of
  Cardiology / American Heart Association Joint Committee on Clinical Practice
  Guidelines (Joglar et al., writing committee); Circulation; 2024.
- **Citation:** Circulation 2024;149(1):e1-e156
- **Kind:** guideline
- **Status:** current
- **Coverage:** `cardiovascular`
- **Locator convention:** `sec<N>.<M>:<section-slug>` or `rec<N>` — guideline
  sections are numbered (e.g. `sec4.2`); numbered recommendations are cited as
  `rec<N>`. Examples: `sec4.2:anticoagulation`, `rec12`.
- **Added by:** rmems
- **Rationale:** Current ACC/AHA-class guideline for atrial fibrillation; the
  cardiology guideline-body anchor for rhythm and anticoagulation
  recommendations (contract §7: edition/year named).

### `ada-standards-diabetes-2025`

- **Bibliographic identity:** *Standards of Care in Diabetes—2025* — American
  Diabetes Association Professional Practice Committee; Diabetes Care; 2025.
- **Citation:** Diabetes Care 2025;48(Suppl. 1)
- **Access date:** 2026-11-20 (living document — updated continuously through
  the year)
- **Kind:** guideline
- **Status:** current
- **Coverage:** `endocrine`
- **Locator convention:** `sec<N>:<title-slug>@YYYY-MM-DD` — the Standards
  are organized into numbered sections; cite the section number and its title
  slug. This is a living document (contract §7): every locator must carry the
  access date of the anchor check as a `@YYYY-MM-DD` suffix. Examples:
  `sec2:diagnosis-classification@2026-11-20`,
  `sec8:pharmacologic-approaches@2026-11-20`.
- **Added by:** rmems
- **Rationale:** The ADA Standards of Care are the gold-standard evidence-based
  guideline for diabetes and prediabetes; a living document updated
  continuously, so the access date is carried per contract §7.

### `adams-victor-neurology-12e`

- **Bibliographic identity:** *Adams and Victor's Principles of Neurology*,
  12th ed. — Allan H. Ropper, Martin A. Samuels, Joshua P. Klein; McGraw
  Hill; 2023.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `neurology`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch37:stroke`, `p.842`.
- **Added by:** rmems
- **Rationale:** The classic neurology text; authoritative for neurologic
  disease mechanisms, clinical manifestations, and management.

### `firestein-kelley-rheumatology-12e`

- **Bibliographic identity:** *Firestein & Kelley's Textbook of
  Rheumatology*, 12th ed. — Gary S. Firestein, Iain B. McInnes, Gary
  Koretzky, Ted Mikuls, Tuhina Neogi, James R. O'Dell (eds.); Elsevier; 2024.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `musculoskeletal`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch89:rheumatoid-arthritis`,
  `p.1904`.
- **Added by:** rmems
- **Rationale:** The comprehensive rheumatology reference; authoritative for
  inflammatory and degenerative musculoskeletal disease.

### `williams-hematology-10e`

- **Bibliographic identity:** *Williams Hematology*, 10th ed. — Kenneth
  Kaushansky, Marshall A. Lichtman, Josef T. Prchal, Marcel M. Levi, Linda J.
  Burns, David C. Linch (eds.); McGraw Hill; 2021.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `hematology_oncology`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch107:iron-deficiency`,
  `p.1401`.
- **Added by:** rmems
- **Rationale:** The landmark hematology reference; authoritative for
  disorders of blood and marrow cells and coagulation.

### `robbins-kumar-basic-pathology-11e`

- **Bibliographic identity:** *Robbins & Kumar Basic Pathology*, 11th ed. —
  Vinay Kumar, Abul K. Abbas, Jon C. Aster, Andrea T. Deyrup, Abhijit Das
  (eds.); Elsevier; 2023.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `cardiovascular`, `respiratory`, `renal`, `gastrointestinal`, `endocrine`, `neurology`, `musculoskeletal`, `hematology_oncology`, `infectious_disease`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch13:myocardial-infarction`,
  `ch15:pneumonia`.
- **Added by:** rmems
- **Rationale:** The standard pathology text; its organ-system chapters give
  the pathologic basis for disease across the pre-clinical systems.

### `sherris-medical-microbiology-7e`

- **Bibliographic identity:** *Sherris Medical Microbiology*, 7th ed. —
  Kenneth J. Ryan, Nafees Ahmad, J. Andrew Alspaugh, W. Lawrence Drew,
  Michael Lagunoff, Paul Pottinger, L. Barth Reller, Megan E. Reller,
  Charles R. Sterling, Scott Weissman (eds.); McGraw Hill; 2018.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `infectious_disease`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch45:staphylococcus-aureus`,
  `p.410`.
- **Added by:** rmems
- **Rationale:** The standard medical microbiology text; authoritative for
  pathogen biology, virulence, and infectious disease mechanisms.

### `katzung-basic-clinical-pharmacology-16e`

- **Bibliographic identity:** *Katzung's Basic and Clinical Pharmacology*,
  16th ed. — Todd W. Vanderah (ed.); McGraw Hill; 2023.
- **Kind:** textbook
- **Status:** current
- **Coverage:** `pharmacology_principles`
- **Locator convention:** `ch<N>:<section-slug>` or `p.<page>` — chapter
  number from the print table of contents, a lowercase hyphenated slug for the
  section, or a stable page number. Examples: `ch10:beta-adrenergic-antagonists`,
  `ch4:pharmacokinetics`.
- **Added by:** rmems
- **Rationale:** The leading pharmacology textbook; authoritative for drug
  mechanisms, pharmacokinetics, and principles of drug action.

### `idsa-iai-update-2024`

- **Bibliographic identity:** *2024 Clinical Practice Guideline Update by the
  Infectious Diseases Society of America on Complicated Intra-abdominal
  Infections: Risk Assessment, Diagnostic Imaging, and Microbiological
  Evaluation in Adults, Children, and Pregnant People* — Infectious Diseases
  Society of America (Bonomo et al., writing committee); Clinical Infectious
  Diseases; 2024.
- **Citation:** Clinical Infectious Diseases 2024;79(Suppl. 3):S81-S87
- **Kind:** guideline
- **Status:** current
- **Coverage:** `infectious_disease`
- **Locator convention:** `rec<N>` or `sec<N>:<title-slug>` — IDSA guidelines
  use numbered recommendations and section headings; cite `rec<N>` for a
  recommendation or the section slug. Examples: `rec3`,
  `sec2:risk-assessment`.
- **Added by:** rmems
- **Rationale:** Current IDSA evidence-based guideline update for complicated
  intra-abdominal infections; the infectious-disease guideline-body anchor
  (contract §7: edition/year named).
