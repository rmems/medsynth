//! Parsing and enforcement of the `docs/teachers.md` pin registry.
//!
//! Fail closed: an unpinned teacher, an incomplete entry, a research-only
//! classification, an unapproved family, or terms that do not affirm
//! distillation and synthetic-data rights all refuse the teacher lane before a
//! client is constructed or a prompt is dispatched.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use medsynth_schema::teacher_pin::{
    TeacherPinEntry, ensure_unique_pin_ids, parse_teacher_pin_entries,
};

use crate::hash::{normalize_newlines, sha256_str, text_file_sha256};
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
    /// Informational: the service the pin describes. The pipeline never calls
    /// it — the external fulfillment harness owns the endpoint and the key.
    pub endpoint: String,
    /// The env var the external harness reads its credential from. The
    /// pipeline never reads it — the field is validated so the pin document
    /// stays an accurate dispatch description.
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
        let entries = parse_teacher_pin_entries(&text);
        ensure_unique_pin_ids(&entries)
            .map_err(|e| TeacherError::PinParse(format!("{}: {e}", path.display())))?;
        let mut pins = BTreeMap::new();
        for entry in entries {
            let pin = parse_pin(&entry);
            pins.insert(pin.pin_id.clone(), pin);
        }
        let file_sha256 = sha256_str(&normalize_newlines(&text));
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
        require_dispatch_env(pin)?;
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

/// Endpoint hosts the external harness may dispatch to. The pin document is
/// what the harness follows, so a pin pointing anywhere else is refused —
/// the credential the harness holds must never be aimed at an untrusted host.
const TRUSTED_ENDPOINT_HOSTS: &[&str] = &["api.deepseek.com", "integrate.api.nvidia.com"];

fn require_trusted_endpoint(pin: &TeacherPin) -> Result<(), TeacherError> {
    let endpoint = pin.endpoint.to_ascii_lowercase();
    if endpoint.starts_with("https://")
        && TRUSTED_ENDPOINT_HOSTS.iter().any(|host| {
            endpoint[8..].starts_with(host) && endpoint[8 + host.len()..].starts_with('/')
        })
    {
        return Ok(());
    }
    Err(TeacherError::UntrustedEndpoint {
        endpoint: pin.endpoint.clone(),
    })
}

/// `api_key_env` documents the variable the harness reads its key from; it
/// must look like a provider API-key variable, not an unrelated credential
/// name a doc edit could sneak in.
fn require_dispatch_env(pin: &TeacherPin) -> Result<(), TeacherError> {
    let name = pin.api_key_env.as_str();
    let plausible = name.ends_with("_API_KEY")
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_');
    if plausible {
        return Ok(());
    }
    Err(TeacherError::InvalidCredentialEnv {
        env_var: name.to_owned(),
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

fn parse_pin(entry: &TeacherPinEntry) -> TeacherPin {
    let get = |key: &str| entry.field(key).unwrap_or("").to_owned();
    let lane = match get("lane").to_ascii_lowercase().as_str() {
        "teacher" => Some(PinLane::Teacher),
        "research_only" | "research-only" => Some(PinLane::ResearchOnly),
        _ => None,
    };
    TeacherPin {
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
        permits_distillation: entry.bool_field("permits_distillation"),
        permits_synthetic_data: entry.bool_field("permits_synthetic_data"),
        snapshot_date: get("snapshot_date"),
        notes: get("notes"),
        entry_text: entry.entry_text.clone(),
        entry_sha256: entry.entry_sha256.clone(),
    }
}
