//! Parsing teacher output into typed records.
//!
//! Teachers return content-only JSON. The pipeline stamps the record id,
//! taxonomy, versions, and provenance; this keeps teachers from setting
//! status or claiming authority over correctness.

use medsynth_schema::{
    Anchor, AnchoredClaim, AnchoredText, CaseVignetteRecord, McqOption, McqRationale, McqRecord,
    OptionLabel, Provenance, SchemaVersion, SourceId, TutoringExampleRecord, TutoringStyle,
    VerificationStatus, VignetteQuestion,
};
use serde::Deserialize;

use crate::config::TargetCell;
use crate::format::Format;
use crate::record::GeneratedRecord;
use crate::teacher::{TeacherError, TeacherPin};

/// Content-level anchor supplied by a teacher.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorJson {
    pub source_id: String,
    pub locator: String,
}

/// A claim block with anchors.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimJson {
    pub text: String,
    pub anchors: Vec<AnchorJson>,
}

/// A text block whose anchors are optional in the record type.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextJson {
    pub text: String,
    #[serde(default)]
    pub anchors: Vec<AnchorJson>,
}

/// One MCQ option from a teacher.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionJson {
    pub label: String,
    pub text: String,
}

/// Teacher content for an MCQ.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McqContent {
    pub stem: String,
    pub anchors: Vec<AnchorJson>,
    pub options: Vec<OptionJson>,
    pub proposed_key: String,
    pub key_explanation: ClaimJson,
    pub strongest_distractor: ClaimJson,
}

/// Teacher content for a case vignette.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VignetteContent {
    pub presentation: String,
    pub findings: Vec<String>,
    pub questions: Vec<VignetteQuestionJson>,
    pub reasoning_chain: Vec<TextJson>,
    pub teaching_points: Vec<ClaimJson>,
}

/// One vignette question.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VignetteQuestionJson {
    pub question: String,
    pub answer: ClaimJson,
}

/// Teacher content for a tutoring example.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TutoringContent {
    pub learner_prompt: TextJson,
    pub tutor_response: TutorResponseJson,
}

/// The tutor response block.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TutorResponseJson {
    pub style: String,
    pub text: String,
    pub anchors: Vec<AnchorJson>,
}

/// Parsed teacher content, tagged by format.
#[derive(Debug, Clone)]
pub enum TeacherContent {
    Mcq(McqContent),
    CaseVignette(VignetteContent),
    Tutoring(TutoringContent),
}

/// Strip an optional markdown fence and parse the content object.
pub fn parse_completion_json(
    format: Format,
    content: &str,
) -> Result<TeacherContent, TeacherError> {
    let json_text = extract_json(content)?;
    match format {
        Format::Mcq => serde_json::from_str::<McqContent>(&json_text)
            .map(TeacherContent::Mcq)
            .map_err(|e| TeacherError::Parse(format!("mcq content: {e}"))),
        Format::CaseVignette => serde_json::from_str::<VignetteContent>(&json_text)
            .map(TeacherContent::CaseVignette)
            .map_err(|e| TeacherError::Parse(format!("case_vignette content: {e}"))),
        Format::TutoringExample => serde_json::from_str::<TutoringContent>(&json_text)
            .map(TeacherContent::Tutoring)
            .map_err(|e| TeacherError::Parse(format!("tutoring_example content: {e}"))),
    }
}

fn extract_json(content: &str) -> Result<String, TeacherError> {
    let text = content.trim();
    if let Some(rest) = text.strip_prefix("```") {
        let rest = rest
            .strip_prefix("json")
            .or_else(|| rest.strip_prefix("JSON"))
            .unwrap_or(rest);
        let rest = rest.trim_start();
        let Some(end) = rest.rfind("```") else {
            return Err(TeacherError::Parse(
                "unterminated markdown fence in teacher output".to_owned(),
            ));
        };
        return Ok(rest[..end].trim().to_owned());
    }
    if text.starts_with('{') && text.ends_with('}') {
        return Ok(text.to_owned());
    }
    // Some providers prepend a short sentence despite the prompt. Fail closed
    // unless the response still contains exactly one JSON object.
    let Some(start) = text.find('{') else {
        return Err(TeacherError::Parse(
            "teacher output contains no JSON object".to_owned(),
        ));
    };
    let Some(end) = text.rfind('}') else {
        return Err(TeacherError::Parse(
            "teacher output contains no closing JSON brace".to_owned(),
        ));
    };
    if end < start {
        return Err(TeacherError::Parse(
            "teacher output has malformed JSON boundaries".to_owned(),
        ));
    }
    Ok(text[start..=end].to_owned())
}

/// Build the record for a teacher content object.
pub fn build_record(
    format: Format,
    content: TeacherContent,
    cell: &TargetCell,
    id: medsynth_schema::RecordId,
    pin: &TeacherPin,
    prompt_version: &str,
) -> Result<GeneratedRecord, TeacherError> {
    let provenance = teacher_provenance(pin, prompt_version);
    match (format, content) {
        (Format::Mcq, TeacherContent::Mcq(content)) => build_mcq(content, cell, id, provenance),
        (Format::CaseVignette, TeacherContent::CaseVignette(content)) => {
            build_vignette(content, cell, id, provenance)
        }
        (Format::TutoringExample, TeacherContent::Tutoring(content)) => {
            build_tutoring(content, cell, id, provenance)
        }
        _ => Err(TeacherError::Parse(
            "teacher content does not match the target format".to_owned(),
        )),
    }
}

fn teacher_provenance(pin: &TeacherPin, prompt_version: &str) -> Provenance {
    Provenance {
        generator: "teacher-pipeline-v1".to_owned(),
        model: Some(pin.variant.clone()),
        prompt_version: Some(prompt_version.to_owned()),
        template_version: None,
    }
}

fn build_mcq(
    content: McqContent,
    cell: &TargetCell,
    id: medsynth_schema::RecordId,
    provenance: Provenance,
) -> Result<GeneratedRecord, TeacherError> {
    let key = parse_label(&content.proposed_key)?;
    let options = content
        .options
        .iter()
        .map(|option| {
            Ok(McqOption {
                label: parse_label(&option.label)?,
                text: option.text.clone(),
                anchors: Vec::new(),
            })
        })
        .collect::<Result<Vec<_>, TeacherError>>()?;
    Ok(GeneratedRecord::Mcq(McqRecord {
        schema_version: SchemaVersion,
        id,
        domain: cell.domain,
        subdomain: cell.subdomain,
        tier: cell.tier,
        stem: AnchoredText {
            text: content.stem,
            anchors: to_anchors(&content.anchors)?,
        },
        options,
        key,
        rationale: McqRationale {
            key_explanation: to_claim(content.key_explanation)?,
            strongest_distractor: to_claim(content.strongest_distractor)?,
        },
        provenance,
        verification_status: VerificationStatus::Unverified,
        labels: Vec::new(),
        taxonomy_version: crate::TAXONOMY_VERSION.to_owned(),
        contract_version: Some(crate::CONTRACT_VERSION.to_owned()),
    }))
}

fn build_vignette(
    content: VignetteContent,
    cell: &TargetCell,
    id: medsynth_schema::RecordId,
    provenance: Provenance,
) -> Result<GeneratedRecord, TeacherError> {
    let questions = content
        .questions
        .into_iter()
        .map(|question| {
            Ok(VignetteQuestion {
                question: question.question,
                answer: to_claim(question.answer)?,
            })
        })
        .collect::<Result<Vec<_>, TeacherError>>()?;
    let reasoning_chain = content
        .reasoning_chain
        .into_iter()
        .map(|step| {
            Ok(AnchoredText {
                text: step.text,
                anchors: to_anchors(&step.anchors)?,
            })
        })
        .collect::<Result<Vec<_>, TeacherError>>()?;
    let teaching_points = content
        .teaching_points
        .into_iter()
        .map(to_claim)
        .collect::<Result<Vec<_>, TeacherError>>()?;
    Ok(GeneratedRecord::CaseVignette(CaseVignetteRecord {
        schema_version: SchemaVersion,
        id,
        domain: cell.domain,
        subdomain: cell.subdomain,
        tier: cell.tier,
        presentation: content.presentation,
        findings: content.findings,
        questions,
        reasoning_chain,
        teaching_points,
        provenance,
        verification_status: VerificationStatus::Unverified,
        labels: Vec::new(),
        taxonomy_version: crate::TAXONOMY_VERSION.to_owned(),
        contract_version: Some(crate::CONTRACT_VERSION.to_owned()),
    }))
}

fn build_tutoring(
    content: TutoringContent,
    cell: &TargetCell,
    id: medsynth_schema::RecordId,
    provenance: Provenance,
) -> Result<GeneratedRecord, TeacherError> {
    let style = match content.tutor_response.style.to_ascii_lowercase().as_str() {
        "socratic" => TutoringStyle::Socratic,
        "explanatory" => TutoringStyle::Explanatory,
        other => {
            return Err(TeacherError::Parse(format!(
                "unknown tutoring style `{other}`"
            )));
        }
    };
    Ok(GeneratedRecord::Tutoring(TutoringExampleRecord {
        schema_version: SchemaVersion,
        id,
        domain: cell.domain,
        subdomain: cell.subdomain,
        tier: cell.tier,
        learner_prompt: AnchoredText {
            text: content.learner_prompt.text,
            anchors: to_anchors(&content.learner_prompt.anchors)?,
        },
        tutor_response: medsynth_schema::TutorResponse {
            style,
            text: content.tutor_response.text,
            anchors: to_anchors(&content.tutor_response.anchors)?,
        },
        provenance,
        verification_status: VerificationStatus::Unverified,
        labels: Vec::new(),
        taxonomy_version: crate::TAXONOMY_VERSION.to_owned(),
        contract_version: Some(crate::CONTRACT_VERSION.to_owned()),
    }))
}

fn to_claim(claim: ClaimJson) -> Result<AnchoredClaim, TeacherError> {
    Ok(AnchoredClaim {
        text: claim.text,
        anchors: to_anchors(&claim.anchors)?,
    })
}

fn to_anchors(anchors: &[AnchorJson]) -> Result<Vec<Anchor>, TeacherError> {
    anchors
        .iter()
        .map(|anchor| {
            if anchor.locator.trim().is_empty() {
                return Err(TeacherError::Parse(
                    "teacher output contains a blank locator".to_owned(),
                ));
            }
            let source_id = SourceId::new(&anchor.source_id).map_err(|e| {
                TeacherError::Parse(format!(
                    "teacher output cites invalid source id `{}`: {e}",
                    anchor.source_id
                ))
            })?;
            Ok(Anchor {
                source_id,
                locator: anchor.locator.clone(),
            })
        })
        .collect()
}

fn parse_label(label: &str) -> Result<OptionLabel, TeacherError> {
    match label.trim().to_ascii_uppercase().as_str() {
        "A" => Ok(OptionLabel::A),
        "B" => Ok(OptionLabel::B),
        "C" => Ok(OptionLabel::C),
        "D" => Ok(OptionLabel::D),
        "E" => Ok(OptionLabel::E),
        other => Err(TeacherError::Parse(format!(
            "invalid option label `{other}`; expected A-E"
        ))),
    }
}
