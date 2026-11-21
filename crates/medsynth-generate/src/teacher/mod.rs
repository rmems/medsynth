//! The teacher lane: pinned model drafting, prompt construction, response
//! parsing, and fail-closed startup.

mod client;
mod parse;
mod pin;
mod prompt;

pub use client::{
    RecordedTeacherClient, TeacherClient, TeacherCompletion, TeacherRequest, completion_tokens,
    fulfillment_key, parse_chat_completion,
};
pub use parse::{RecordStamp, TeacherContent, build_record, parse_completion_json};
pub use pin::{PinLane, TeacherPin, TeacherPinRegistry};
pub use prompt::{AnchorPoint, BuiltPrompt, build_prompt};

use std::fmt;

/// A teacher-lane refusal or failure.
#[derive(Debug, thiserror::Error)]
pub enum TeacherError {
    #[error("teacher lane refused: no pin `{0}` in docs/teachers.md")]
    Unpinned(String),
    #[error("teacher lane refused: pin `{pin_id}` is incomplete; missing {missing:?}")]
    IncompletePin {
        pin_id: String,
        missing: Vec<&'static str>,
    },
    #[error(
        "teacher lane refused: pin `{0}` is research-only or belongs to a research-only family"
    )]
    ResearchOnly(String),
    #[error(
        "teacher lane refused: family `{family}` is not an approved teacher family (DeepSeek or Nemotron 3 only)"
    )]
    IneligibleFamily { family: String },
    #[error(
        "teacher lane refused: pin `{pin_id}` does not affirm terms permitting distillation and synthetic-data generation"
    )]
    TermsDoNotPermit { pin_id: String },
    #[error("teacher lane refused: teachers pin file error: {0}")]
    PinParse(String),
    #[error(
        "teacher lane refused: teacher cells need a fulfillment directory (--teacher-responses) of recorded teacher responses; the pipeline never calls teacher APIs itself"
    )]
    MissingFulfillmentDir,
    #[error("teacher request failed: {0}")]
    Transport(String),
    #[error("teacher response could not be parsed: {0}")]
    Parse(String),
    #[error("teacher output exceeds the {limit}-token budget (got {actual})")]
    OverBudget { limit: u32, actual: u32 },
    #[error("teacher prompt exceeds the {limit}-token budget (got {actual})")]
    PromptOverBudget { limit: u32, actual: u32 },
    #[error("teacher response omitted `choices[0].message.content`")]
    MissingContent,
}

/// A teacher that passed every pin check. Credentials belong to the external
/// harness that fulfills requests — the pipeline never holds or reads them.
#[derive(Clone)]
pub struct PreparedTeacher {
    pin: TeacherPin,
}

impl PreparedTeacher {
    pub fn pin(&self) -> &TeacherPin {
        &self.pin
    }
}

impl fmt::Debug for PreparedTeacher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PreparedTeacher")
            .field("pin_id", &self.pin.pin_id)
            .field("variant", &self.pin.variant)
            .finish()
    }
}

/// Run the fail-closed pin checks.
pub fn prepare(
    registry: &TeacherPinRegistry,
    pin_id: &str,
) -> Result<PreparedTeacher, TeacherError> {
    let pin = registry.ensure_teacher(pin_id)?.clone();
    Ok(PreparedTeacher { pin })
}
