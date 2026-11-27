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
    AnchoredClaim, AnchoredText, Domain, McqRationale, McqRecord, Provenance, SchemaVersion,
    SourceId, Subdomain, Tier, VerificationStatus,
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

/// One catalog entry. The macro keeps each fact terse while the `Fact`
/// literal stays the single construction site.
macro_rules! fact {
    (
        $id:literal, $domain:expr, $subdomain:expr, $tier:expr,
        $stem:literal, $correct:literal,
        [$($distractor:literal),+ $(,)?],
        $explain:literal, $refute:literal,
        [$($anchor:expr),+ $(,)?] $(,)?
    ) => {
        Fact {
            id: $id,
            domain: $domain,
            subdomain: $subdomain,
            tier: $tier,
            stem: $stem,
            correct: $correct,
            distractors: [$($distractor),+],
            key_explanation: $explain,
            distractor_refutation: $refute,
            anchors: &[$($anchor),+],
        }
    };
}

/// The fact catalog. Every anchor cites an approved registry source; the
/// registry check happens again during self-validation.
pub const FACTS: &[Fact] = &[
    fact!(
        "fact.alveolar_epithelium",
        Domain::Bbf,
        Subdomain::AnimalStructure,
        Tier::Foundation,
        "Which epithelial type forms the thin alveolar lining that minimizes the diffusion distance for gas exchange?",
        "Simple squamous epithelium",
        [
            "Simple cuboidal epithelium",
            "Stratified squamous epithelium",
            "Pseudostratified columnar epithelium",
        ],
        "Simple squamous epithelium is a single layer of flattened cells; its thinness supports rapid diffusion across the alveolar wall.",
        "Simple cuboidal and columnar epithelia are thicker and specialized for secretion or absorption, and stratified squamous epithelium is multilayered and protective rather than optimized for diffusion.",
        [
            ("boundless-anatomy-physiology-2018", "sec21.4A:lungs"),
            (
                "boundless-anatomy-physiology-2018",
                "sec4.2B:types-of-epithelial-tissue"
            ),
        ]
    ),
    fact!(
        "fact.ethnocentrism",
        Domain::Psb,
        Subdomain::CultureIdentity,
        Tier::Foundation,
        "A researcher evaluates a community's healing practices by the standards of the researcher's own culture and labels them inferior. This judgment best illustrates:",
        "Ethnocentrism",
        ["Cultural relativism", "Cultural universal", "Assimilation"],
        "Ethnocentrism is the practice of judging another culture by the standards of one's own culture.",
        "Cultural relativism instead evaluates a culture in its own terms, a cultural universal is a trait common across cultures, and assimilation describes minority integration into a dominant culture.",
        [
            (
                "boundless-sociology-2018",
                "sec3.1E:ethnocentrism-and-cultural-relativism",
            ),
            ("boundless-sociology-2018", "sec3.1C:cultural-universals",),
            ("boundless-sociology-2018", "sec10.4D:assimilation"),
        ]
    ),
    fact!(
        "fact.diaphragm",
        Domain::PreClinical,
        Subdomain::Respiratory,
        Tier::Foundation,
        "Which muscle is the principal driver of quiet inspiration in a healthy adult?",
        "Diaphragm",
        [
            "External intercostal muscles",
            "Rectus abdominis",
            "Sternocleidomastoid muscles",
        ],
        "The diaphragm contracts during quiet inspiration, flattening and descending to increase thoracic volume and lower intrathoracic pressure.",
        "External intercostals assist by elevating the ribs, while the rectus abdominis and sternocleidomastoid are accessory or expiratory muscles rather than the principal driver of quiet inspiration.",
        [("boundless-anatomy-physiology-2018", "sec21.5B:inspiration",)]
    ),
    fact!(
        "fact.pulmonary_artery",
        Domain::PreClinical,
        Subdomain::Cardiovascular,
        Tier::Foundation,
        "Which vessel carries deoxygenated blood from the right ventricle toward the lungs?",
        "Pulmonary artery",
        ["Pulmonary vein", "Aorta", "Superior vena cava"],
        "The pulmonary artery leaves the right ventricle and carries deoxygenated blood to the lungs for gas exchange.",
        "The pulmonary vein returns oxygenated blood to the left atrium, the aorta carries oxygenated blood from the left ventricle, and the superior vena cava returns deoxygenated blood to the right atrium.",
        [
            (
                "boundless-anatomy-physiology-2018",
                "sec21.4C:blood-supply-to-the-lungs",
            ),
            (
                "boundless-anatomy-physiology-2018",
                "sec17.1E:great-vessels-of-the-heart"
            ),
        ]
    ),
    fact!(
        "fact.osteoclast",
        Domain::PreClinical,
        Subdomain::Musculoskeletal,
        Tier::Foundation,
        "Which bone cell resorbs bone matrix during remodeling?",
        "Osteoclast",
        ["Osteoblast", "Osteocyte", "Chondrocyte"],
        "Osteoclasts are multinucleated cells that resorb bone matrix during remodeling.",
        "Osteoblasts form new bone matrix, osteocytes maintain bone tissue, and chondrocytes produce cartilage.",
        [(
            "boundless-anatomy-physiology-2018",
            "sec6.4C:bone-remodeling"
        )]
    ),
    fact!(
        "fact.posterior_pituitary",
        Domain::PreClinical,
        Subdomain::Endocrine,
        Tier::Foundation,
        "Which hormone is released from the posterior pituitary?",
        "Antidiuretic hormone",
        [
            "Thyroid-stimulating hormone",
            "Prolactin",
            "Adrenocorticotropic hormone",
        ],
        "The posterior pituitary releases antidiuretic hormone and oxytocin, which are produced in the hypothalamus.",
        "Thyroid-stimulating hormone, prolactin, and adrenocorticotropic hormone are released from the anterior pituitary.",
        [(
            "boundless-anatomy-physiology-2018",
            "sec15.7D:the-posterior-pituitary",
        )]
    ),
    fact!(
        "fact.mitochondrion_atp",
        Domain::Bbf,
        Subdomain::CellFunction,
        Tier::Foundation,
        "Which organelle generates the majority of ATP in an aerobic eukaryotic cell?",
        "Mitochondrion",
        ["Nucleus", "Smooth endoplasmic reticulum", "Lysosome"],
        "Mitochondria carry out oxidative phosphorylation, which supplies most of the ATP used by aerobic cells.",
        "The nucleus stores genetic information, the smooth endoplasmic reticulum synthesizes lipids and detoxifies, and lysosomes degrade material — none is the primary ATP source.",
        [(
            "mol-bio-cell-7e",
            "ch14:electron-transport-oxidative-phosphorylation",
        )]
    ),
    fact!(
        "fact.si_unit_force",
        Domain::Cpf,
        Subdomain::Mechanics,
        Tier::Foundation,
        "What is the SI unit of force?",
        "Newton",
        ["Joule", "Coulomb", "Watt"],
        "The newton is the SI unit of force, defined as one kilogram-meter per second squared.",
        "The joule is the SI unit of energy, the coulomb is the unit of electric charge, and the watt is the unit of power.",
        [(
            "ostp-introductory-physics-2019",
            "sec2.2:units-and-dimensions"
        )]
    ),
    fact!(
        "fact.fundamental_attribution",
        Domain::Psb,
        Subdomain::Psychology,
        Tier::Foundation,
        "An observer attributes a coworker's missed deadline to laziness while overlooking the coworker's heavy workload. The observer's judgment best illustrates:",
        "Fundamental attribution error",
        ["Self-serving bias", "Halo effect", "Just-world hypothesis"],
        "The fundamental attribution error is the tendency to overestimate dispositional explanations and underestimate situational ones for others' behavior.",
        "Self-serving bias applies to explanations of one's own behavior, the halo effect generalizes a single positive trait to the whole person, and the just-world hypothesis assumes people get what they deserve.",
        [("myers-psychology-14e", "ch18:attribution-theory")]
    ),
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
    let (options, key) = crate::procedural::shuffled_options(
        fact.correct,
        fact.distractors.iter().map(|d| (*d).to_owned()),
        rng,
    )
    .map_err(|e| format!("template `{}`: {e}", fact.id))?;

    let key_anchor_count = if fact.id == "fact.ethnocentrism" {
        1
    } else {
        fact.anchors.len()
    };
    let key_anchors = to_anchors(&fact.anchors[..key_anchor_count])?;
    let anchors = to_anchors(fact.anchors)?;
    let record = McqRecord {
        schema_version: SchemaVersion,
        id,
        domain: cell.domain,
        subdomain: cell.subdomain,
        tier: cell.tier,
        stem: AnchoredText {
            text: fact.stem.to_owned(),
            anchors: key_anchors.clone(),
        },
        options,
        key,
        rationale: McqRationale {
            key_explanation: AnchoredClaim {
                text: format!(
                    "Option {key} ({}) is the best answer. {}",
                    fact.correct, fact.key_explanation
                ),
                anchors: key_anchors,
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
