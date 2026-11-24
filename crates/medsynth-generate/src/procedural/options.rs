//! Shared MCQ option assembly.
//!
//! The key is always derived from where the correct text lands after the
//! seeded shuffle. No template or teacher asserts a label directly.

use medsynth_schema::{McqOption, OptionLabel};

use crate::rng::SeededRng;

pub(crate) const LABELS: [OptionLabel; 4] = [
    OptionLabel::A,
    OptionLabel::B,
    OptionLabel::C,
    OptionLabel::D,
];

/// Shuffle the correct answer among its distractors and return both the
/// options and the label carrying the correct answer.
pub(crate) fn build_options(
    correct: &str,
    distractors: impl IntoIterator<Item = String>,
    rng: &mut SeededRng,
) -> Result<(Vec<McqOption>, OptionLabel), String> {
    let mut texts = Vec::with_capacity(4);
    texts.push(correct.to_owned());
    texts.extend(distractors);
    rng.shuffle(&mut texts);

    let mut key = None;
    let mut options = Vec::with_capacity(texts.len());
    for (text, label) in texts.into_iter().zip(LABELS) {
        if text == correct {
            key = Some(label);
        }
        options.push(McqOption {
            label,
            text,
            anchors: Vec::new(),
        });
    }
    let key = key.ok_or_else(|| "computed correct answer is missing".to_owned())?;
    Ok((options, key))
}
