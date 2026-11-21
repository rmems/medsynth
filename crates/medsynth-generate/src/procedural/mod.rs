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
pub const TEMPLATE_VERSION: &str = "1";

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
    if let Some(fact) = Fact::find(template_id) {
        return if fact.matches_cell(cell) {
            Ok(())
        } else {
            Err(format!(
                "template `{template_id}` targets a different domain/subdomain/tier than cell `{}`",
                cell.label()
            ))
        };
    }
    if let Some(template) = CalcTemplate::find(template_id) {
        return if template.matches_cell(cell) {
            Ok(())
        } else {
            Err(format!(
                "template `{template_id}` targets a different domain/subdomain/tier than cell `{}`",
                cell.label()
            ))
        };
    }
    Err(format!("unknown procedural template `{template_id}`"))
}

/// All procedural template ids, for the CLI listing command.
pub fn template_ids() -> Vec<&'static str> {
    let mut ids: Vec<&'static str> = FACTS.iter().map(|f| f.id).collect();
    ids.extend(CALC_TEMPLATES.iter().map(|t| t.id));
    ids
}
