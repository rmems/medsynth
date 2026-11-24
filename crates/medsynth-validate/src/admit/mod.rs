//! Verification & admission pipeline — the correctness gate between draft
//! batches and `data/<format>/`.
//!
//! Draft records enter at `unverified` and pass through, in order:
//!
//! 0. **intake** — the batch's rights envelope: teacher-lane records need a
//!    complete manifest envelope (record id, teacher variant, teachers.md
//!    pin, content hash); procedural records carry seed + template id.
//! 1. **schema** — the shared record schema check (`run_schema_check`).
//! 2. **answer key** — a solver re-derives the key; the solver must be a
//!    different model and run than the drafter, and must be pinned in the
//!    run config (fail-closed when unpinable or unpinned).
//! 3. **source anchors** — every source anchor resolves against the current
//!    registry and every fact-bearing claim verifies as supported; stale
//!    anchor receipts hold the record for re-anchoring rather than
//!    rejecting it.
//! 4. **distractors** — every MCQ distractor is wrong for a statable reason.
//! 5. **review** — an agent-driven accuracy review over a deterministic
//!    sample of the batch; a failed review holds the record.
//!
//! Semantic work (solving, claim support, review) is fulfilled externally
//! through request/response files — the crate writes `<key>.request.json`
//! into the configured fulfillment dir and reads `<key>.response.json`
//! back. Missing or unusable responses hold the record (a transient
//! outage), never silently pass it.
//!
//! Admitted records — and only admitted records — are written to
//! `data/<format>/<batch-id>.jsonl` with `verification_status: "admitted"`
//! and the run's `contract_version`. Every run emits a JSON admission log
//! with per-record outcomes, per-stage reject/hold counts, and the run's
//! pinned solver/verifier/reviewer ids.

mod batch;
mod claims;
mod config;
mod fulfill;
mod hash;
mod pipeline;
mod registry;

pub use config::{AgentPin, RunConfig, SolverPin, load_run_config};
pub use pipeline::{
    AdmissionOptions, AdmissionReport, Counts, Outcome, RecordOutcome, ReviewReport, ReviewResult,
    Stage, run,
};
