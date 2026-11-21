//! Calculation templates.
//!
//! A calculation template samples parameters, puts those same parameters in
//! the stem, and computes the key from them. The key is therefore derived,
//! not asserted: changing a param changes both the stem and the correct
//! option together.

use medsynth_schema::{
    AnchoredClaim, AnchoredText, Domain, McqRationale, McqRecord, Provenance, SchemaVersion,
    Subdomain, Tier, VerificationStatus,
};

use crate::config::TargetCell;
use crate::procedural::TEMPLATE_VERSION;
use crate::procedural::facts::to_anchors;
use crate::record::GeneratedRecord;
use crate::rng::SeededRng;

/// Metadata for one executable calculation template.
pub struct CalcTemplate {
    pub id: &'static str,
    pub domain: Domain,
    pub subdomain: Subdomain,
    pub tier: Tier,
}

/// The calculation templates.
pub const CALC_TEMPLATES: &[CalcTemplate] = &[
    CalcTemplate {
        id: "calc.newtons_second_law",
        domain: Domain::Cpf,
        subdomain: Subdomain::Mechanics,
        tier: Tier::Application,
    },
    CalcTemplate {
        id: "calc.half_life",
        domain: Domain::PreClinical,
        subdomain: Subdomain::PharmacologyPrinciples,
        tier: Tier::Application,
    },
];

impl CalcTemplate {
    pub fn find(id: &str) -> Option<&'static CalcTemplate> {
        CALC_TEMPLATES.iter().find(|t| t.id == id)
    }

    pub fn matches_cell(&self, cell: &TargetCell) -> bool {
        self.domain == cell.domain && self.subdomain == cell.subdomain && self.tier == cell.tier
    }
}

/// One sampled calculation: stem, derived answer, distractors, and the
/// parameters used to compute them.
#[derive(Debug, Clone)]
pub struct CalcInstance {
    pub stem: String,
    pub correct_value: u64,
    pub correct_text: String,
    pub distractors: Vec<String>,
    pub key_explanation: String,
    pub distractor_refutation: String,
    pub anchors: &'static [(&'static str, &'static str)],
    /// Parameter name → value, for tests and audit.
    pub params: Vec<(&'static str, u64)>,
}

/// Evaluate a template with a seeded stream.
pub fn evaluate(template_id: &str, rng: &mut SeededRng) -> Result<CalcInstance, String> {
    match template_id {
        "calc.newtons_second_law" => Ok(newtons_second_law(rng)),
        "calc.half_life" => Ok(half_life(rng)),
        other => Err(format!("unknown calculation template `{other}`")),
    }
}

fn newtons_second_law(rng: &mut SeededRng) -> CalcInstance {
    let mass = rng.range_inclusive(2, 12);
    let acceleration = rng.range_inclusive(2, 9);
    let force = mass * acceleration;
    let correct_value = u64::from(acceleration);
    let correct_text = format!("{acceleration} m/s^2");
    let distractors = three_distractors(
        correct_value,
        vec![
            u64::from(acceleration + 1),
            u64::from(acceleration * 2),
            u64::from(force + mass),
        ],
    )
    .into_iter()
    .map(|v| format!("{v} m/s^2"))
    .collect();
    CalcInstance {
        stem: format!(
            "A net force of {force} N acts on a cart of mass {mass} kg on a frictionless track. What is the cart's acceleration?"
        ),
        correct_value,
        correct_text,
        distractors,
        key_explanation: format!(
            "Newton's second law gives a = F_net / m. With F_net = {force} N and m = {mass} kg, a = {force} / {mass} = {acceleration} m/s^2."
        ),
        distractor_refutation: format!(
            "Doubling the acceleration, adding force and mass ({force} + {mass}), or adding one to the acceleration does not follow Newton's second law; acceleration is net force divided by mass."
        ),
        anchors: &[(
            "openstax-university-physics-2016",
            "vol1:ch5:newtons-second-law",
        )],
        params: vec![
            ("mass_kg", u64::from(mass)),
            ("net_force_n", u64::from(force)),
            ("acceleration_mps2", u64::from(acceleration)),
        ],
    }
}

fn half_life(rng: &mut SeededRng) -> CalcInstance {
    let initial = *rng.choose(&[400u64, 800, 1200, 1600]);
    let half_life_hours = *rng.choose(&[2u64, 4, 6, 8]);
    let half_lives = u64::from(rng.range_inclusive(1, 3));
    let elapsed = half_life_hours * half_lives;
    let remaining = initial >> half_lives;
    let distractors = three_distractors(
        remaining,
        vec![remaining * 3 / 2, remaining / 2, remaining * 2],
    )
    .into_iter()
    .map(|v| format!("{v} mg"))
    .collect();
    CalcInstance {
        stem: format!(
            "A fictional patient receives a single {initial} mg dose of a drug whose elimination half-life is {half_life_hours} hours. Assuming first-order elimination, approximately how much drug remains {elapsed} hours after the dose?"
        ),
        correct_value: remaining,
        correct_text: format!("{remaining} mg"),
        distractors,
        key_explanation: format!(
            "After n = elapsed / half-life = {elapsed} / {half_life_hours} = {half_lives} half-lives, first-order elimination leaves initial x (1/2)^n = {initial} mg x (1/2)^{half_lives} = {remaining} mg."
        ),
        distractor_refutation: "Using one and a half times, one half, or twice the correct remainder corresponds to using the wrong number of half-lives; each half-life halves the remaining amount.".to_owned(),
        anchors: &[(
            "katzung-basic-clinical-pharmacology-16e",
            "ch2:pharmacokinetics",
        )],
        params: vec![
            ("dose_mg", initial),
            ("half_life_h", half_life_hours),
            ("half_lives", half_lives),
            ("elapsed_h", elapsed),
        ],
    }
}

fn three_distractors(correct: u64, candidates: Vec<u64>) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::with_capacity(3);
    for candidate in candidates {
        push_unique(&mut out, candidate, correct);
    }
    let mut bump = 1;
    while out.len() < 3 {
        push_unique(&mut out, correct + bump, correct);
        bump += 1;
    }
    out
}

fn push_unique(out: &mut Vec<u64>, candidate: u64, correct: u64) {
    if candidate != correct && !out.contains(&candidate) {
        out.push(candidate);
    }
}

/// Build a calculation MCQ. Options are shuffled; the key is the label of the
/// option carrying the computed value.
pub(crate) fn build_calc_record(
    template: &CalcTemplate,
    cell: &TargetCell,
    id: medsynth_schema::RecordId,
    rng: &mut SeededRng,
) -> Result<GeneratedRecord, String> {
    if !template.matches_cell(cell) {
        return Err(format!(
            "template `{}` does not match cell `{}`",
            template.id,
            cell.label()
        ));
    }
    let instance = evaluate(template.id, rng)?;
    let (options, key) = crate::procedural::shuffled_options(
        &instance.correct_text,
        instance.distractors.iter().cloned(),
        rng,
    )
    .map_err(|e| format!("template `{}`: {e}", template.id))?;

    let anchors = to_anchors(instance.anchors)?;
    let record = McqRecord {
        schema_version: SchemaVersion,
        id,
        domain: cell.domain,
        subdomain: cell.subdomain,
        tier: cell.tier,
        stem: AnchoredText {
            text: instance.stem,
            anchors: anchors.clone(),
        },
        options,
        key,
        rationale: McqRationale {
            key_explanation: AnchoredClaim {
                text: format!(
                    "Option {key} is the best answer. {}",
                    instance.key_explanation
                ),
                anchors: anchors.clone(),
            },
            strongest_distractor: AnchoredClaim {
                text: instance.distractor_refutation,
                anchors,
            },
        },
        provenance: Provenance {
            generator: format!("template:{}", template.id),
            model: None,
            prompt_version: None,
            template_version: Some(format!("{}@{TEMPLATE_VERSION}", template.id)),
        },
        verification_status: VerificationStatus::Unverified,
        labels: Vec::new(),
        taxonomy_version: crate::TAXONOMY_VERSION.to_owned(),
        contract_version: Some(crate::CONTRACT_VERSION.to_owned()),
    };
    Ok(GeneratedRecord::Mcq(record))
}
