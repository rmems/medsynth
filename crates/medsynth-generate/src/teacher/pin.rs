//! Parsing and enforcement of the `docs/teachers.md` pin registry.
//!
//! Fail closed: an unpinned teacher, an incomplete entry, a research-only
//! classification, an unapproved family, or terms that do not affirm
//! distillation and synthetic-data rights all refuse the teacher lane before a
//! client is constructed or a prompt is dispatched.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::hash::{sha256_str, text_file_sha256};
use crate::teacher::TeacherError;

/// How a pin may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinLane {
    /// May draft records for the generation pipeline.
    Teacher,
    /// Research-only; never drafts records.
    ResearchOnly,
}

/// One `### Pin:` entry from `docs/teachers.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeacherPin {
    pub pin_id: String,
    pub provider: String,
    pub family: String,
    pub variant: String,
    pub model_id: String,
    pub endpoint: String,
    pub api_key_env: String,
    pub lane: Option<PinLane>,
    pub terms_document: String,
    pub terms_version: String,
    pub terms_date: String,
    pub terms_url: String,
    pub terms_clause: String,
    pub permits_distillation: Option<bool>,
    pub permits_synthetic_data: Option<bool>,
    pub snapshot_date: String,
    pub notes: String,
    /// Exact block text, from the `### Pin:` heading through the last field.
    pub entry_text: String,
    /// SHA-256 of `entry_text`.
    pub entry_sha256: String,
}

impl TeacherPin {
    /// Fields that are empty or absent. An entry with any missing field is
    /// incomplete and the lane refuses to start.
    pub fn missing_fields(&self) -> Vec<&'static str> {
        let mut missing = Vec::new();
        for (name, value) in [
            ("pin_id", self.pin_id.as_str()),
            ("provider", self.provider.as_str()),
            ("family", self.family.as_str()),
            ("variant", self.variant.as_str()),
            ("model_id", self.model_id.as_str()),
            ("endpoint", self.endpoint.as_str()),
            ("api_key_env", self.api_key_env.as_str()),
            ("terms_document", self.terms_document.as_str()),
            ("terms_version", self.terms_version.as_str()),
            ("terms_date", self.terms_date.as_str()),
            ("terms_url", self.terms_url.as_str()),
            ("terms_clause", self.terms_clause.as_str()),
            ("snapshot_date", self.snapshot_date.as_str()),
        ] {
            if value.trim().is_empty() {
                missing.push(name);
            }
        }
        if self.lane.is_none() {
            missing.push("lane");
        }
        if self.permits_distillation.is_none() {
            missing.push("permits_distillation");
        }
        if self.permits_synthetic_data.is_none() {
            missing.push("permits_synthetic_data");
        }
        missing
    }

    pub fn is_complete(&self) -> bool {
        self.missing_fields().is_empty()
    }
}

/// All pins parsed from the pin file.
#[derive(Debug, Clone)]
pub struct TeacherPinRegistry {
    pub path: PathBuf,
    pub file_sha256: String,
    pins: BTreeMap<String, TeacherPin>,
}

impl TeacherPinRegistry {
    /// Parse `docs/teachers.md`.
    pub fn load(path: &Path) -> Result<Self, TeacherError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| TeacherError::PinParse(format!("cannot read {}: {e}", path.display())))?;
        let mut pins = BTreeMap::new();
        for entry_text in pin_blocks(&text) {
            let pin = parse_pin(&entry_text)?;
            if pin.pin_id.trim().is_empty() {
                return Err(TeacherError::PinParse(
                    "a Pin block has a blank pin_id".to_owned(),
                ));
            }
            if pins.insert(pin.pin_id.clone(), pin).is_some() {
                return Err(TeacherError::PinParse(
                    "duplicate pin_id in docs/teachers.md".to_owned(),
                ));
            }
        }
        let file_sha256 = text_file_sha256(path)
            .map_err(|e| TeacherError::PinParse(format!("cannot hash {}: {e}", path.display())))?;
        Ok(Self {
            path: path.to_path_buf(),
            file_sha256,
            pins,
        })
    }

    /// Assemble a registry from already-parsed pins.
    ///
    /// The generation path always uses [`TeacherPinRegistry::load`]; this
    /// constructor exists for tests and tooling that need to exercise the
    /// envelope gate with a deliberately incomplete pin.
    pub fn from_pins(path: PathBuf, pins: Vec<TeacherPin>) -> Result<Self, TeacherError> {
        let mut map = BTreeMap::new();
        for pin in pins {
            if map.insert(pin.pin_id.clone(), pin).is_some() {
                return Err(TeacherError::PinParse(
                    "duplicate pin_id in assembled pin list".to_owned(),
                ));
            }
        }
        let file_sha256 = text_file_sha256(&path).unwrap_or_else(|_| sha256_str(""));
        Ok(Self {
            path,
            file_sha256,
            pins: map,
        })
    }

    pub fn get(&self, pin_id: &str) -> Option<&TeacherPin> {
        self.pins.get(pin_id)
    }

    pub fn pins(&self) -> impl Iterator<Item = &TeacherPin> {
        self.pins.values()
    }

    /// The gate. Returns the pin only if every requirement is met.
    pub fn ensure_teacher(&self, pin_id: &str) -> Result<&TeacherPin, TeacherError> {
        let pin = self
            .pins
            .get(pin_id)
            .ok_or_else(|| TeacherError::Unpinned(pin_id.to_owned()))?;
        require_complete(pin)?;
        require_teacher_lane(pin)?;
        require_approved_family(pin)?;
        require_trusted_endpoint(pin)?;
        require_credential_env(pin)?;
        require_terms(pin)?;
        Ok(pin)
    }
}

/// Names that mark a family or provider as research-only (never drafting).
const RESEARCH_ONLY_KEYS: &[&str] = &["claude", "anthropic", "grok", "xai"];

/// Teacher families approved under the standing distillation rule.
const APPROVED_FAMILIES: &[&str] = &["deepseek", "nvidia-nemotron", "nemotron"];

fn is_research_only_name(pin: &TeacherPin) -> bool {
    [&pin.family, &pin.provider].iter().any(|name| {
        let lowered = name.to_ascii_lowercase();
        RESEARCH_ONLY_KEYS.iter().any(|key| lowered.contains(key))
    })
}

fn require_complete(pin: &TeacherPin) -> Result<(), TeacherError> {
    let missing = pin.missing_fields();
    if missing.is_empty() {
        return Ok(());
    }
    Err(TeacherError::IncompletePin {
        pin_id: pin.pin_id.clone(),
        missing,
    })
}

fn require_teacher_lane(pin: &TeacherPin) -> Result<(), TeacherError> {
    if pin.lane != Some(PinLane::Teacher) {
        return Err(TeacherError::ResearchOnly(pin.pin_id.clone()));
    }
    if is_research_only_name(pin) {
        return Err(TeacherError::ResearchOnly(pin.pin_id.clone()));
    }
    Ok(())
}

fn require_approved_family(pin: &TeacherPin) -> Result<(), TeacherError> {
    let family = pin.family.to_ascii_lowercase();
    if APPROVED_FAMILIES.contains(&family.as_str()) {
        return Ok(());
    }
    Err(TeacherError::IneligibleFamily { family })
}

/// The pin's endpoint must be the family's approved HTTPS host — credentials
/// must never be sent to an arbitrary endpoint named in the pin file.
fn require_trusted_endpoint(pin: &TeacherPin) -> Result<(), TeacherError> {
    let family = pin.family.to_ascii_lowercase();
    let host = endpoint_host(&pin.endpoint);
    let trusted = matches!(
        (family.as_str(), host),
        ("deepseek", Some("api.deepseek.com"))
            | (
                "nvidia-nemotron" | "nemotron",
                Some("integrate.api.nvidia.com")
            )
    );
    if trusted {
        return Ok(());
    }
    Err(TeacherError::UntrustedEndpoint {
        pin_id: pin.pin_id.clone(),
        endpoint: pin.endpoint.clone(),
    })
}

/// The lowercase host of an `https://` URL, or `None` when the endpoint is
/// not HTTPS (never send credentials over plaintext).
fn endpoint_host(endpoint: &str) -> Option<&str> {
    let rest = endpoint.strip_prefix("https://")?;
    rest.split(['/', '?'])
        .next()?
        .split(':')
        .next()
        .filter(|&h| !h.is_empty())
}

/// Credentials are read only from `MEDSYNTH_TEACHER_*` variables, so a pin
/// cannot point the pipeline at an unrelated secret in the environment.
fn require_credential_env(pin: &TeacherPin) -> Result<(), TeacherError> {
    if pin.api_key_env.starts_with("MEDSYNTH_TEACHER_") {
        return Ok(());
    }
    Err(TeacherError::InvalidCredentialEnv {
        pin_id: pin.pin_id.clone(),
        env_var: pin.api_key_env.clone(),
    })
}

fn require_terms(pin: &TeacherPin) -> Result<(), TeacherError> {
    if pin.permits_distillation != Some(true) || pin.permits_synthetic_data != Some(true) {
        return Err(TeacherError::TermsDoNotPermit {
            pin_id: pin.pin_id.clone(),
        });
    }
    Ok(())
}

fn pin_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines() {
        let is_pin_heading = line.starts_with("### Pin:") || line.starts_with("## Pin:");
        let is_other_heading =
            !is_pin_heading && (line.starts_with("## ") || line.starts_with("### "));
        if is_pin_heading || is_other_heading {
            if !current.is_empty() {
                blocks.push(join_block(&current));
            }
            current.clear();
        }
        if is_pin_heading || (!is_other_heading && !current.is_empty()) {
            current.push(line);
        }
    }
    if !current.is_empty() {
        blocks.push(join_block(&current));
    }
    blocks
}

fn join_block(lines: &[&str]) -> String {
    lines.join("\n").trim().to_owned()
}

fn parse_pin(entry_text: &str) -> Result<TeacherPin, TeacherError> {
    let mut fields = BTreeMap::new();
    for line in entry_text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("- **") else {
            continue;
        };
        let Some((key, value)) = rest.split_once(":**") else {
            continue;
        };
        fields.insert(key.trim().to_ascii_lowercase(), clean_value(value));
    }
    let get = |key: &str| fields.get(key).cloned().unwrap_or_default();
    let lane = match get("lane").to_ascii_lowercase().as_str() {
        "teacher" => Some(PinLane::Teacher),
        "research_only" | "research-only" => Some(PinLane::ResearchOnly),
        _ => None,
    };
    Ok(TeacherPin {
        pin_id: get("pin_id"),
        provider: get("provider"),
        family: get("family"),
        variant: get("variant"),
        model_id: get("model_id"),
        endpoint: get("endpoint"),
        api_key_env: get("api_key_env"),
        lane,
        terms_document: get("terms_document"),
        terms_version: get("terms_version"),
        terms_date: get("terms_date"),
        terms_url: get("terms_url"),
        terms_clause: get("terms_clause"),
        permits_distillation: parse_bool(&get("permits_distillation")),
        permits_synthetic_data: parse_bool(&get("permits_synthetic_data")),
        snapshot_date: get("snapshot_date"),
        notes: get("notes"),
        entry_text: entry_text.to_owned(),
        entry_sha256: sha256_str(entry_text),
    })
}

fn clean_value(value: &str) -> String {
    let value = value.trim();
    let value = value.strip_prefix('`').unwrap_or(value);
    let value = value.strip_suffix('`').unwrap_or(value);
    value.trim().to_owned()
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" => Some(true),
        "no" | "false" | "0" => Some(false),
        _ => None,
    }
}
