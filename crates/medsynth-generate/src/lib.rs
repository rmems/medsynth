//! Candidate-record generation for medsynth.
//!
//! The crate runs two lanes:
//!
//! * **Procedural** — deterministic templates whose answer keys are computed
//!   from the same parameters that appear in the stem. Seeded with
//!   [`rand_pcg`], so the same seed and config reproduce a batch byte for byte.
//! * **Teacher** — model drafting for case vignettes, tutoring examples, and
//!   non-templatable MCQs. The lane fails closed unless the selected teacher is
//!   pinned in `docs/teachers.md`, its terms permit distillation and
//!   synthetic-data generation, and its credential is present in the
//!   environment. Teachers draft surface and structure only; every draft stays
//!   `unverified` for the verification pipeline (#9).
//!
//! Every record is validated against `schemas/<format>.v1.json`, deserialized
//! into its `medsynth-schema` type, and passed through cross-field
//! `validate()` before it is written. A failing draft is dropped and counted,
//! never written. The batch manifest carries the seed, config, teacher pin
//! reference, per-cell counts, per-record SHA-256 hashes, rights envelopes for
//! teacher drafts, and anchor receipts for every anchor.

/// Version of `docs/taxonomy.md` this generator writes into records.
pub const TAXONOMY_VERSION: &str = "0.1";

/// Version of `docs/contract.md` this generator writes into records.
pub const CONTRACT_VERSION: &str = "0.1";

pub mod batch;
pub mod config;
pub mod error;
pub mod format;
pub mod gold;
pub mod hash;
pub mod manifest;
pub mod matrix;
pub mod procedural;
pub mod record;
pub mod registry;
pub mod rng;
pub mod teacher;
pub mod validate;

pub use batch::{BatchOutput, BatchPlan, ExecuteOptions, execute, load_plan};
pub use config::{Lane, RunConfig, TargetCell, TeacherSelection};
pub use error::GenerateError;
pub use format::Format;
pub use matrix::{CellKey, CoverageMatrix};
pub use record::GeneratedRecord;
pub use registry::SourceRegistry;
pub use teacher::{TeacherError, TeacherPin, TeacherPinRegistry};
