//! The admission run configuration — the pins the gate fails closed without.
//!
//! Semantic work in the pipeline (the stage-2 solve, the stage-3 claim
//! support and stage-4 distractor screens, and the stage-5 accuracy review)
//! is done by external agents, never by this binary. The run config pins who
//! those agents are so the admission log can name them and so an unpinned
//! gate refuses to run rather than silently passing records.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The stage-2 solver pin. The solver is a separate model pass — it must
/// never be the model or run that drafted a record (checked per draft via
/// the rights envelope from #8).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolverPin {
    /// Identifier of the solver run, recorded in the admission log and
    /// required to match the run id each response file reports.
    pub run_id: String,
    /// The model the solver ran — compared against each draft's rights
    /// envelope and `provenance.model`.
    pub model_id: String,
    /// The `docs/teachers.md` pin the solver ran under, when it was a
    /// pinned teacher. Compared against the envelope's `pin_id`.
    #[serde(default)]
    pub pin_id: Option<String>,
}

/// Identity of the stage-3/4 verifier run and the stage-5 reviewer run.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentPin {
    /// Identifier of the run, recorded in the admission log and required to
    /// match the run id each response file reports.
    pub run_id: String,
    /// The model the run used, when applicable — compared against each
    /// draft's rights envelope the same way the solver's is.
    #[serde(default)]
    pub model_id: Option<String>,
}

/// The admission run configuration (JSON).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunConfig {
    /// Directory shared with the external fulfillers. The pipeline writes
    /// `<key>.<kind>.request.json` files there and reads
    /// `<key>.<kind>.response.json` back, exactly like the teacher lane.
    pub fulfillment_dir: PathBuf,
    /// Stage-2 solver pin — required when the batch holds keyed records
    /// (`mcq`, `case_vignette`); the gate refuses to start without it.
    #[serde(default)]
    pub solver: Option<SolverPin>,
    /// Stage-3/4 verifier identity — required for any non-empty batch.
    #[serde(default)]
    pub verifier: Option<AgentPin>,
    /// Stage-5 reviewer identity — required for any non-empty batch.
    #[serde(default)]
    pub reviewer: Option<AgentPin>,
    /// Stage-5 sample rate (default 0.10; the sample is never smaller than
    /// 5 records, or the whole batch when it holds fewer).
    #[serde(default = "default_sample_rate")]
    pub sample_rate: f64,
    /// The contract version admitted records are stamped with — the version
    /// in force for this run (contract §9).
    pub contract_version: String,
}

fn default_sample_rate() -> f64 {
    0.10
}

/// Load and check a run config. `needs_solver` / `needs_verifier` /
/// `needs_reviewer` come from the batch's shape: a solver pin is required
/// only when keyed records exist, but verifier and reviewer pins are
/// required for any non-empty batch.
pub fn load_run_config(path: &Path) -> Result<RunConfig, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read run config {}: {e}", path.display()))?;
    let config: RunConfig =
        serde_json::from_str(&text).map_err(|e| format!("run config {}: {e}", path.display()))?;
    config.check_shape(path)?;
    Ok(config)
}

impl RunConfig {
    /// Field-level checks that do not depend on the batch.
    fn check_shape(&self, path: &Path) -> Result<(), String> {
        let label = || format!("run config {}", path.display());
        if self.contract_version.trim().is_empty() {
            return Err(format!("{}: contract_version is blank", label()));
        }
        // Contract §9: the gate verifies against the currently effective
        // version — a run configured for a superseded contract refuses to
        // start rather than stamp records with stale rules.
        if self.contract_version.trim() != medsynth_schema::CONTRACT_VERSION {
            return Err(format!(
                "{}: contract_version `{}` is not the gate's `{}`",
                label(),
                self.contract_version,
                medsynth_schema::CONTRACT_VERSION
            ));
        }
        if !(self.sample_rate > 0.0 && self.sample_rate <= 1.0) {
            return Err(format!(
                "{}: sample_rate {} is outside (0, 1]",
                label(),
                self.sample_rate
            ));
        }
        self.check_pins_shape(&label())?;
        self.check_distinct_runs(&label())
    }

    fn check_pins_shape(&self, label: &str) -> Result<(), String> {
        if let Some(solver) = &self.solver
            && (solver.run_id.trim().is_empty() || solver.model_id.trim().is_empty())
        {
            return Err(format!("{label}: solver pin requires run_id and model_id"));
        }
        for (name, agent) in [("verifier", &self.verifier), ("reviewer", &self.reviewer)] {
            if let Some(agent) = agent
                && agent.run_id.trim().is_empty()
            {
                return Err(format!("{label}: {name} run_id is blank"));
            }
        }
        Ok(())
    }

    /// The review run must be a third agent — never the drafting run and
    /// never the stage-2 solver run.
    fn check_distinct_runs(&self, label: &str) -> Result<(), String> {
        let ids: Vec<&str> = [
            self.solver.as_ref().map(|s| s.run_id.as_str()),
            self.verifier.as_ref().map(|a| a.run_id.as_str()),
            self.reviewer.as_ref().map(|a| a.run_id.as_str()),
        ]
        .into_iter()
        .flatten()
        .collect();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != ids.len() {
            return Err(format!(
                "{label}: solver, verifier, and reviewer run_ids must be distinct"
            ));
        }
        Ok(())
    }

    /// Fail-closed pin checks against the batch's shape. Missing pins are
    /// run-config errors — the gate refuses to start rather than admit
    /// records nobody verified.
    pub fn check_pins(
        &self,
        needs_solver: bool,
        needs_verifier: bool,
        batch_empty: bool,
    ) -> Result<(), String> {
        if batch_empty {
            return Err("batch holds no records".to_owned());
        }
        if needs_solver
            && self
                .solver
                .as_ref()
                .is_none_or(|s| s.run_id.trim().is_empty() || s.model_id.trim().is_empty())
        {
            return Err(
                "batch holds keyed records but the run config pins no stage-2 solver".to_owned(),
            );
        }
        if needs_verifier && self.verifier.is_none() {
            return Err(
                "batch holds claim-bearing records but the run config pins no stage-3/4 verifier"
                    .to_owned(),
            );
        }
        if self.reviewer.is_none() {
            return Err("run config pins no stage-5 reviewer".to_owned());
        }
        Ok(())
    }
}
