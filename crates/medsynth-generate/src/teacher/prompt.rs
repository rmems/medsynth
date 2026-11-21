//! Prompt construction for the teacher lane.
//!
//! Prompts carry the taxonomy cell, the approved anchor points the teacher may
//! cite, and one or more gold records as style exemplars. They forbid
//! reproducing or paraphrasing known exam items, require fictional patients,
//! and make clear that the teacher drafts surface and structure only — the
//! verification pipeline decides correctness.

use medsynth_schema::Tier;
use serde_json::Value;
use tiktoken_rs::cl100k_base_singleton;

use crate::config::{TargetCell, TeacherSelection};
use crate::format::Format;
use crate::teacher::TeacherError;

/// One approved source point offered to the teacher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchorPoint {
    pub source_id: String,
    pub locator: String,
    /// A short anchored fact, when the pipeline has one. A locator-only point
    /// is still an approved registry locator, but the teacher must use it only
    /// where it supports the claim.
    pub fact: String,
}

/// A built prompt with its token count under the cl100k tokenizer used for
/// budgeting.
#[derive(Debug, Clone)]
pub struct BuiltPrompt {
    pub system: String,
    pub user: String,
    pub prompt_version: String,
    pub token_count: u32,
}

const SYSTEM_PROMPT: &str = "You draft candidate medical-education records for medsynth. \
You draft surface prose and structure only; you never certify correctness. A separate \
verification pipeline will re-derive every answer key and check every cited passage. \
Build only from the supplied taxonomy cell and the supplied anchor points. Do not use \
outside clinical facts. Do not reproduce, paraphrase, or adapt any MCAT, USMLE, COMLEX, \
or commercial question-bank item, and do not use material from a published case report \
or textbook vignette. Every patient, clinician, and institution must be fictional: no real \
names, dates, locations, record numbers, or identifiable details. Do not give medical \
advice or instructions for a real person. Cite only the source_id and locator pairs in the \
supplied anchor list; never invent a citation. Return exactly one JSON object matching the \
requested shape, with no markdown fence and no commentary.";

/// Build the system and user prompts for one cell.
pub fn build_prompt(
    cell: &TargetCell,
    anchors: &[AnchorPoint],
    exemplars: &[&Value],
    teacher: &TeacherSelection,
) -> Result<BuiltPrompt, TeacherError> {
    let mut user = String::new();
    user.push_str("## Taxonomy cell\n\n");
    user.push_str(&format!(
        "- domain: {}\n- subdomain: {}\n- format: {}\n- tier: {}\n",
        domain_slug(cell),
        cell.subdomain_slug(),
        cell.format.slug(),
        tier_slug(cell.tier)
    ));
    user.push_str(&format!(
        "- tier definition: {}\n\n",
        tier_definition(cell.format, cell.tier)
    ));

    user.push_str("## Approved anchor points\n\n");
    user.push_str(
        "Cite only these exact pairs. Use an anchor only when it supports the attached claim; \
         if no supplied anchor supports a claim, omit the claim rather than invent a citation.\n\n",
    );
    for anchor in anchors {
        if anchor.fact.trim().is_empty() {
            user.push_str(&format!(
                "- source_id: {} | locator: {} | fact: (registry-approved locator; use only if it supports the claim)\n",
                anchor.source_id, anchor.locator
            ));
        } else {
            user.push_str(&format!(
                "- source_id: {} | locator: {} | fact: {}\n",
                anchor.source_id, anchor.locator, anchor.fact
            ));
        }
    }
    if anchors.is_empty() {
        user.push_str("- (none supplied; do not make clinical claims)\n");
    }
    user.push('\n');

    if !exemplars.is_empty() {
        user.push_str("## Gold exemplars (style and structure only — do not copy content)\n\n");
        for exemplar in exemplars {
            let id = exemplar
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("<gold>");
            user.push_str(&format!("### {id}\n"));
            user.push_str(&serde_json::to_string(exemplar).unwrap_or_default());
            user.push_str("\n\n");
        }
    }

    user.push_str("## Output shape\n\n");
    user.push_str(output_shape(cell.format));
    user.push_str("\n## Additional requirements\n\n");
    user.push_str(
        "- Fictional patients and invented observations only; no PHI and no real cases.\n\
         - Board-style format is fine; board content is prohibited.\n\
         - Every claim block carries at least one supplied anchor; empty anchors are allowed \
         only for pure pedagogy or invented patient observations.\n\
         - Do not add fields beyond the shape above.\n\
         - Keep the whole object under the output token budget.\n",
    );
    user.push_str(&format!(
        "- Output token budget: {}.\n",
        teacher.max_output_tokens
    ));

    let token_count = count_tokens(SYSTEM_PROMPT) + count_tokens(&user);
    if token_count > teacher.max_prompt_tokens {
        return Err(TeacherError::PromptOverBudget {
            limit: teacher.max_prompt_tokens,
            actual: token_count,
        });
    }
    Ok(BuiltPrompt {
        system: SYSTEM_PROMPT.to_owned(),
        user,
        prompt_version: teacher.prompt_version.clone(),
        token_count,
    })
}

/// Token count under cl100k, used as the budgeting tokenizer for both prompt
/// and completion limits.
pub fn count_tokens(text: &str) -> u32 {
    cl100k_base_singleton()
        .encode_with_special_tokens(text)
        .len() as u32
}

fn output_shape(format: Format) -> &'static str {
    match format {
        Format::Mcq => {
            "```json\n\
{\n\
  \"stem\": \"string\",\n\
  \"anchors\": [{\"source_id\": \"supplied id\", \"locator\": \"supplied locator\"}],\n\
  \"options\": [\n\
    {\"label\": \"A\", \"text\": \"string\"},\n\
    {\"label\": \"B\", \"text\": \"string\"},\n\
    {\"label\": \"C\", \"text\": \"string\"},\n\
    {\"label\": \"D\", \"text\": \"string\"}\n\
  ],\n\
  \"proposed_key\": \"A\",\n\
  \"key_explanation\": {\"text\": \"string\", \"anchors\": [{\"source_id\": \"...\", \"locator\": \"...\"}]},\n\
  \"strongest_distractor\": {\"text\": \"string\", \"anchors\": [{\"source_id\": \"...\", \"locator\": \"...\"}]}\n\
}\n\
```\n\
`proposed_key` is a proposal only; a verifier will re-derive the key. Supply 4 or 5 \
options with unique labels A–E.\n"
        }
        Format::CaseVignette => {
            "```json\n\
{\n\
  \"presentation\": \"string\",\n\
  \"findings\": [\"string\"],\n\
  \"questions\": [{\"question\": \"string\", \"answer\": {\"text\": \"string\", \"anchors\": [{\"source_id\": \"...\", \"locator\": \"...\"}]}}],\n\
  \"reasoning_chain\": [{\"text\": \"string\", \"anchors\": [{\"source_id\": \"...\", \"locator\": \"...\"}]}],\n\
  \"teaching_points\": [{\"text\": \"string\", \"anchors\": [{\"source_id\": \"...\", \"locator\": \"...\"}]}]\n\
}\n\
```\n"
        }
        Format::TutoringExample => {
            "```json\n\
{\n\
  \"learner_prompt\": {\"text\": \"string\", \"anchors\": []},\n\
  \"tutor_response\": {\"style\": \"socratic\" or \"explanatory\", \"text\": \"string\", \"anchors\": [{\"source_id\": \"...\", \"locator\": \"...\"}]}\n\
}\n\
```\n"
        }
    }
}

fn domain_slug(cell: &TargetCell) -> &'static str {
    crate::matrix::domain_slug(cell.domain)
}

fn tier_slug(tier: Tier) -> &'static str {
    crate::matrix::tier_slug(tier)
}

fn tier_definition(format: Format, tier: Tier) -> &'static str {
    match (format, tier) {
        (Format::Mcq, Tier::Foundation) => {
            "Tests recall or direct comprehension of a single anchored fact or concept; one step, no scenario inference."
        }
        (Format::Mcq, Tier::Application) => {
            "Applies a mechanism or relationship to a brief self-contained scenario; one inference step separates stem from key."
        }
        (Format::Mcq, Tier::Reasoning) => {
            "Integrates two or more concepts or discriminates competing mechanisms; the key is defensible only through chained reasoning."
        }
        (Format::CaseVignette, Tier::Foundation) => {
            "Short uncomplicated fictional vignette; asks for a basic identification resolvable by pattern-matching to one teaching point."
        }
        (Format::CaseVignette, Tier::Application) => {
            "Connects presented findings to the underlying mechanism or principle; findings map cleanly onto one system or concept."
        }
        (Format::CaseVignette, Tier::Reasoning) => {
            "Integrates findings across systems or time, prioritizes competing interpretations, or reconciles a finding that fits more than one explanation."
        }
        (Format::TutoringExample, Tier::Foundation) => {
            "Walks through one concept or fact with the reasoning explicit at each step; poses no novel problem."
        }
        (Format::TutoringExample, Tier::Application) => {
            "Guides the learner step by step through a typical self-contained problem, naming each inference."
        }
        (Format::TutoringExample, Tier::Reasoning) => {
            "Models multi-step reasoning on a non-trivial problem, including a plausible wrong turn and where the reasoning could go wrong."
        }
    }
}
