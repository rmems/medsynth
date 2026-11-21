//! Fact-recall templates.
//!
//! Each entry is a single anchored teaching fact. The stem is the fact's
//! question, the correct option is the fact's answer, and the key is derived
//! by looking up where that answer landed after the seeded shuffle — it is
//! never hardcoded. Distractors are anchored through the rationale's
//! refutation (contract §4).

use crate::config::TargetCell;
use crate::procedural::TEMPLATE_VERSION;
use crate::record::GeneratedRecord;
use crate::rng::SeededRng;
use medsynth_schema::{
    AnchoredClaim, AnchoredText, Domain, McqOption, McqRationale, McqRecord, OptionLabel,
    Provenance, SchemaVersion, SourceId, Subdomain, Tier, VerificationStatus,
};

/// One anchored fact-recall template.
pub struct Fact {
    pub id: &'static str,
    pub domain: Domain,
    pub subdomain: Subdomain,
    pub tier: Tier,
    pub stem: &'static str,
    pub correct: &'static str,
    pub distractors: [&'static str; 3],
    pub key_explanation: &'static str,
    pub distractor_refutation: &'static str,
    pub anchors: &'static [(&'static str, &'static str)],
}

/// The fact catalog. Every anchor cites an approved registry source; the
/// registry check happens again during self-validation.
pub const FACTS: &[Fact] = &[
    Fact {
        id: "fact.alveolar_epithelium",
        domain: Domain::Bbf,
        subdomain: Subdomain::AnimalStructure,
        tier: Tier::Foundation,
        stem: "Which epithelial type forms the thin alveolar lining that minimizes the diffusion distance for gas exchange?",
        correct: "Simple squamous epithelium",
        distractors: [
            "Simple cuboidal epithelium",
            "Stratified squamous epithelium",
            "Pseudostratified columnar epithelium",
        ],
        key_explanation: "Simple squamous epithelium is a single layer of flattened cells; its thinness supports rapid diffusion across the alveolar wall.",
        distractor_refutation: "Simple cuboidal and columnar epithelia are thicker and specialized for secretion or absorption, and stratified squamous epithelium is multilayered and protective rather than optimized for diffusion.",
        anchors: &[
            ("openstax-anatomy-physiology-2e", "sec22.2:the-lungs"),
            ("openstax-anatomy-physiology-2e", "sec4.2:epithelial-tissue"),
        ],
    },
    Fact {
        id: "fact.ethnocentrism",
        domain: Domain::Psb,
        subdomain: Subdomain::CultureIdentity,
        tier: Tier::Foundation,
        stem: "A researcher evaluates a community's healing practices by the standards of the researcher's own culture and labels them inferior. This judgment best illustrates:",
        correct: "Ethnocentrism",
        distractors: ["Cultural relativism", "Xenocentrism", "Assimilation"],
        key_explanation: "Ethnocentrism is the practice of judging another culture by the standards of one's own culture.",
        distractor_refutation: "Cultural relativism instead evaluates a culture by its own standards, xenocentrism favors another culture over one's own, and assimilation describes adoption of a dominant culture's practices.",
        anchors: &[(
            "openstax-introduction-sociology-3e",
            "ch3:elements-of-culture",
        )],
    },
    Fact {
        id: "fact.diaphragm",
        domain: Domain::PreClinical,
        subdomain: Subdomain::Respiratory,
        tier: Tier::Foundation,
        stem: "Which muscle is the principal driver of quiet inspiration in a healthy adult?",
        correct: "Diaphragm",
        distractors: [
            "External intercostal muscles",
            "Rectus abdominis",
            "Sternocleidomastoid muscles",
        ],
        key_explanation: "The diaphragm contracts during quiet inspiration, flattening and descending to increase thoracic volume and lower intrathoracic pressure.",
        distractor_refutation: "External intercostals assist by elevating the ribs, while the rectus abdominis and sternocleidomastoid are accessory or expiratory muscles rather than the principal driver of quiet inspiration.",
        anchors: &[(
            "openstax-anatomy-physiology-2e",
            "sec22.3:the-process-of-breathing",
        )],
    },
    Fact {
        id: "fact.pulmonary_artery",
        domain: Domain::PreClinical,
        subdomain: Subdomain::Cardiovascular,
        tier: Tier::Foundation,
        stem: "Which vessel carries deoxygenated blood from the right ventricle toward the lungs?",
        correct: "Pulmonary artery",
        distractors: ["Pulmonary vein", "Aorta", "Superior vena cava"],
        key_explanation: "The pulmonary artery leaves the right ventricle and carries deoxygenated blood to the lungs for gas exchange.",
        distractor_refutation: "The pulmonary vein returns oxygenated blood to the left atrium, the aorta carries oxygenated blood from the left ventricle, and the superior vena cava returns deoxygenated blood to the right atrium.",
        anchors: &[
            (
                "openstax-anatomy-physiology-2e",
                "sec20.1:structure-and-function-of-blood-vessels",
            ),
            ("openstax-anatomy-physiology-2e", "sec19.1:heart-anatomy"),
        ],
    },
    Fact {
        id: "fact.osteoclast",
        domain: Domain::PreClinical,
        subdomain: Subdomain::Musculoskeletal,
        tier: Tier::Foundation,
        stem: "Which bone cell resorbs bone matrix during remodeling?",
        correct: "Osteoclast",
        distractors: ["Osteoblast", "Osteocyte", "Chondrocyte"],
        key_explanation: "Osteoclasts are multinucleated cells that resorb bone matrix during remodeling.",
        distractor_refutation: "Osteoblasts form new bone matrix, osteocytes maintain bone tissue, and chondrocytes produce cartilage.",
        anchors: &[("openstax-anatomy-physiology-2e", "sec6.3:bone-structure")],
    },
    Fact {
        id: "fact.posterior_pituitary",
        domain: Domain::PreClinical,
        subdomain: Subdomain::Endocrine,
        tier: Tier::Foundation,
        stem: "Which hormone is released from the posterior pituitary?",
        correct: "Antidiuretic hormone",
        distractors: [
            "Thyroid-stimulating hormone",
            "Prolactin",
            "Adrenocorticotropic hormone",
        ],
        key_explanation: "The posterior pituitary releases antidiuretic hormone and oxytocin, which are produced in the hypothalamus.",
        distractor_refutation: "Thyroid-stimulating hormone, prolactin, and adrenocorticotropic hormone are released from the anterior pituitary.",
        anchors: &[(
            "openstax-anatomy-physiology-2e",
            "sec17.3:the-pituitary-gland-and-hypothalamus",
        )],
    },
];

impl Fact {
    pub fn find(id: &str) -> Option<&'static Fact> {
        FACTS.iter().find(|f| f.id == id)
    }

    pub fn matches_cell(&self, cell: &TargetCell) -> bool {
        self.domain == cell.domain && self.subdomain == cell.subdomain && self.tier == cell.tier
    }
}

/// Build a fact-recall MCQ. The key is the label of the option that carries
/// the fact's correct answer after the seeded shuffle.
pub(crate) fn build_fact_record(
    fact: &Fact,
    cell: &TargetCell,
    id: medsynth_schema::RecordId,
    rng: &mut SeededRng,
) -> Result<GeneratedRecord, String> {
    if !fact.matches_cell(cell) {
        return Err(format!(
            "template `{}` does not match cell `{}`",
            fact.id,
            cell.label()
        ));
    }
    let mut texts = Vec::with_capacity(4);
    texts.push(fact.correct.to_owned());
    texts.extend(fact.distractors.iter().map(|d| (*d).to_owned()));
    rng.shuffle(&mut texts);

    let labels = [
        OptionLabel::A,
        OptionLabel::B,
        OptionLabel::C,
        OptionLabel::D,
    ];
    let mut key = None;
    let mut options = Vec::with_capacity(texts.len());
    for (text, label) in texts.into_iter().zip(labels) {
        if text == fact.correct {
            key = Some(label);
        }
        options.push(McqOption {
            label,
            text,
            anchors: Vec::new(),
        });
    }
    let key = key.ok_or_else(|| format!("template `{}`: correct option missing", fact.id))?;

    let anchors = to_anchors(fact.anchors)?;
    let record = McqRecord {
        schema_version: SchemaVersion,
        id,
        domain: cell.domain,
        subdomain: cell.subdomain,
        tier: cell.tier,
        stem: AnchoredText {
            text: fact.stem.to_owned(),
            anchors: anchors.clone(),
        },
        options,
        key,
        rationale: McqRationale {
            key_explanation: AnchoredClaim {
                text: format!(
                    "Option {key} ({}) is the best answer. {}",
                    fact.correct, fact.key_explanation
                ),
                anchors: anchors.clone(),
            },
            strongest_distractor: AnchoredClaim {
                text: format!(
                    "The other options are not the best answer. {}",
                    fact.distractor_refutation
                ),
                anchors,
            },
        },
        provenance: Provenance {
            generator: format!("template:{}", fact.id),
            model: None,
            prompt_version: None,
            template_version: Some(format!("{}@{TEMPLATE_VERSION}", fact.id)),
        },
        verification_status: VerificationStatus::Unverified,
        labels: Vec::new(),
        taxonomy_version: crate::TAXONOMY_VERSION.to_owned(),
        contract_version: Some(crate::CONTRACT_VERSION.to_owned()),
    };
    Ok(GeneratedRecord::Mcq(record))
}

pub(crate) fn to_anchors(
    pairs: &'static [(&'static str, &'static str)],
) -> Result<Vec<medsynth_schema::Anchor>, String> {
    pairs
        .iter()
        .map(|(source, locator)| {
            let source_id =
                SourceId::new(*source).map_err(|e| format!("invalid source id `{source}`: {e}"))?;
            Ok(medsynth_schema::Anchor {
                source_id,
                locator: (*locator).to_owned(),
            })
        })
        .collect()
}
