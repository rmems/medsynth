//! The procedural lane: deterministic templates for templatable MCQs.

mod calc;
mod facts;

pub use calc::{CALC_TEMPLATES, CalcInstance, CalcTemplate, evaluate};
pub use facts::{FACTS, Fact};

use crate::config::TargetCell;
use crate::record::GeneratedRecord;
use crate::rng::SeededRng;

/// Version stamped into `provenance.template_version`. Bump it when a
/// template's text or logic changes.
pub const TEMPLATE_VERSION: &str = "2";

/// Build one procedural draft.
pub fn build(
    template_id: &str,
    cell: &TargetCell,
    id: medsynth_schema::RecordId,
    rng: &mut SeededRng,
) -> Result<GeneratedRecord, String> {
    if let Some(fact) = Fact::find(template_id) {
        return facts::build_fact_record(fact, cell, id, rng);
    }
    if let Some(template) = CalcTemplate::find(template_id) {
        return calc::build_calc_record(template, cell, id, rng);
    }
    Err(format!("unknown procedural template `{template_id}`"))
}

/// Confirm a template exists and targets the configured cell.
pub fn check_template_for_cell(template_id: &str, cell: &TargetCell) -> Result<(), String> {
    let found = Fact::find(template_id)
        .map(|fact| fact.matches_cell(cell))
        .or_else(|| CalcTemplate::find(template_id).map(|template| template.matches_cell(cell)));
    match found {
        Some(true) => Ok(()),
        Some(false) => Err(format!(
            "template `{template_id}` targets a different domain/subdomain/tier than cell `{}`",
            cell.label()
        )),
        None => Err(format!("unknown procedural template `{template_id}`")),
    }
}

/// Shuffle `correct` and `distractors` with the seeded stream and return the
/// labeled options plus the key — the label where the correct text landed.
/// Used by every MCQ builder so the key is always looked up, never asserted.
pub(crate) fn shuffled_options(
    correct: &str,
    distractors: impl Iterator<Item = String>,
    rng: &mut SeededRng,
) -> Result<
    (
        Vec<medsynth_schema::McqOption>,
        medsynth_schema::OptionLabel,
    ),
    String,
> {
    use medsynth_schema::{McqOption, OptionLabel};

    let mut texts = Vec::with_capacity(4);
    texts.push(correct.to_owned());
    texts.extend(distractors);
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
        if text == correct {
            key = Some(label);
        }
        options.push(McqOption {
            label,
            text,
            anchors: Vec::new(),
        });
    }
    let key = key.ok_or_else(|| "computed correct option missing after shuffle".to_owned())?;
    Ok((options, key))
}

/// All procedural template ids, for the CLI listing command.
pub fn template_ids() -> Vec<&'static str> {
    let mut ids: Vec<&'static str> = FACTS.iter().map(|f| f.id).collect();
    ids.extend(CALC_TEMPLATES.iter().map(|t| t.id));
    ids
}
