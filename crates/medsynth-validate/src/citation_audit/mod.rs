//! Citation-support audit (#26): resolve cited passages and record whether an
//! independent adjudicator says each passage supports the claim citing it.
//!
//! The audit separates deterministic retrieval, semantic adjudication, and
//! reporting. OpenStax pages are fetched into a working cache outside `data/`;
//! only passage hashes are reported. Semantic verdicts come from file-based
//! independent adjudicator responses, or from the existing gold review manifest
//! when its quoted evidence still matches the retrieved passage. Fetched
//! passage text is never written under `data/` and never emitted in the report.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::admit::claims::{ClaimBlock, claim_blocks_for};
use crate::{RecordFile, json_files, load_paths};

/// Access policy for one source whose license permits passage text access.
#[derive(Debug, Clone, Deserialize)]
pub struct AccessEntry {
    pub source_id: String,
    pub license: String,
    pub retrieval: String,
    pub book: String,
}

#[derive(Debug, Deserialize)]
struct AccessRegistry {
    sources: Vec<AccessEntry>,
}

/// Inputs for one citation-audit run.
#[derive(Debug, Clone)]
pub struct AuditOptions {
    /// Repository/corpus root used for report-relative record paths.
    pub root: PathBuf,
    /// Record file or directory to audit.
    pub records: PathBuf,
    /// Machine-readable access policy (`docs/citation-access.json`).
    pub access: PathBuf,
    /// Working passage cache. This must be outside `data/`.
    pub cache_dir: PathBuf,
    /// File-based independent-adjudicator exchange directory.
    pub fulfillment_dir: PathBuf,
    /// Optional gold review manifest used as pre-existing independent
    /// adjudication for hash-identical gold records.
    pub gold_review: Option<PathBuf>,
    /// Drafting run id that adjudicators must not equal, when known.
    pub drafting_run_id: Option<String>,
    /// Do not perform network fetches; use only pre-populated cache entries.
    pub offline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Supported,
    Unsupported,
    Unverifiable,
}

impl Verdict {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "supported" => Some(Self::Supported),
            "unsupported" => Some(Self::Unsupported),
            "unverifiable" => Some(Self::Unverifiable),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AnchorVerdict {
    pub record_id: String,
    pub record_path: String,
    pub claim_path: String,
    pub source_id: String,
    pub locator: String,
    pub verdict: Verdict,
    pub passage_sha256: Option<String>,
    pub passage_url: Option<String>,
    pub adjudicator_run_id: Option<String>,
    pub access: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditCounts {
    pub records: usize,
    pub claim_blocks: usize,
    pub anchors: usize,
    pub supported: usize,
    pub unsupported: usize,
    pub unverifiable: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditReport {
    pub audit_version: u32,
    pub counts: AuditCounts,
    /// Records with at least one unsupported anchor. Release assembly must
    /// exclude these ids until the failing citation is re-anchored and audited.
    pub release_blocking_record_ids: Vec<String>,
    /// Sources with no text-access policy. These anchors stay receipt-level by
    /// design and are still represented in `anchors` below.
    pub unverifiable_by_design_sources: Vec<String>,
    pub anchors: Vec<AnchorVerdict>,
}

impl AuditReport {
    pub fn unsupported(&self) -> usize {
        self.counts.unsupported
    }
}

pub(crate) fn sha256_text(text: &str) -> String {
    hex_lower(Sha256::digest(text.as_bytes()))
}

fn hex_lower(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

fn load_access(path: &Path) -> anyhow::Result<BTreeMap<String, AccessEntry>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("cannot read access policy {}", path.display()))?;
    let registry: AccessRegistry = serde_json::from_str(&text)
        .with_context(|| format!("cannot parse access policy {}", path.display()))?;
    Ok(registry
        .sources
        .into_iter()
        .map(|entry| (entry.source_id.clone(), entry))
        .collect())
}

fn discover_records(input: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if input.is_file() {
        return Ok(vec![input.to_path_buf()]);
    }
    if !input.is_dir() {
        bail!("records path {} does not exist", input.display());
    }
    let (mut files, findings) = json_files(input);
    if !findings.is_empty() {
        bail!(
            "cannot enumerate {}: {}",
            input.display(),
            findings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    files.retain(|path| {
        !path.components().any(|c| c.as_os_str() == "near-miss")
            && path.file_name().and_then(|n| n.to_str()) != Some("review-manifest.json")
    });
    Ok(files)
}

fn load_records(root: &Path, input: &Path) -> anyhow::Result<Vec<RecordFile>> {
    let files = discover_records(input)?;
    let root = root.to_path_buf();
    let (records, findings) = load_paths(files, move |p| {
        p.strip_prefix(&root).unwrap_or(p).to_path_buf()
    });
    if !findings.is_empty() {
        bail!(
            "cannot load records: {}",
            findings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    Ok(records)
}

mod adjudication;
mod passages;

use adjudication::{
    GoldAdjudication, GoldReviewManifest, evidence_matches_passage, gold_adjudication,
    load_gold_review, response_verdict, write_request,
};
use passages::{Passage, retrieve_passage};

#[derive(Default)]
struct AuditState {
    anchors: Vec<AnchorVerdict>,
    claim_count: usize,
    receipt_sources: BTreeSet<String>,
}

struct RunContext<'a> {
    options: &'a AuditOptions,
    access: &'a BTreeMap<String, AccessEntry>,
    review: Option<&'a GoldReviewManifest>,
}

struct RecordAudit<'a> {
    record: &'a RecordFile,
    options: &'a AuditOptions,
    access: &'a BTreeMap<String, AccessEntry>,
    gold: Option<GoldAdjudication>,
    drafting_run_id: Option<String>,
}

struct AnchorInput<'a> {
    claim: &'a ClaimBlock,
    source_id: &'a str,
    locator: &'a str,
    access: Option<&'a AccessEntry>,
}

struct PassageEvidence {
    sha256: String,
    url: String,
}

struct AnchorOutcome {
    verdict: Verdict,
    passage: Option<PassageEvidence>,
    run_id: Option<String>,
    access: String,
    detail: String,
}

fn final_verdict(
    context: &RecordAudit<'_>,
    input: &AnchorInput<'_>,
    outcome: AnchorOutcome,
) -> AnchorVerdict {
    AnchorVerdict {
        record_id: context.record.record_id(),
        record_path: context.record.path.display().to_string().replace('\\', "/"),
        claim_path: input.claim.path.clone(),
        source_id: input.source_id.to_owned(),
        locator: input.locator.to_owned(),
        verdict: outcome.verdict,
        passage_sha256: outcome.passage.as_ref().map(|p| p.sha256.clone()),
        passage_url: outcome.passage.as_ref().map(|p| p.url.clone()),
        adjudicator_run_id: outcome.run_id,
        access: outcome.access,
        detail: outcome.detail,
    }
}

fn no_access_verdict(context: &RecordAudit<'_>, input: &AnchorInput<'_>) -> AnchorVerdict {
    final_verdict(
        context,
        input,
        AnchorOutcome {
            verdict: Verdict::Unverifiable,
            passage: None,
            run_id: context.gold.as_ref().map(|gold| gold.run_id.clone()),
            access: "receipt-level-only".to_owned(),
            detail: "source has no license-compatible text-access policy".to_owned(),
        },
    )
}

fn retrieval_error_verdict(
    context: &RecordAudit<'_>,
    input: &AnchorInput<'_>,
    entry: &AccessEntry,
    error: anyhow::Error,
) -> AnchorVerdict {
    final_verdict(
        context,
        input,
        AnchorOutcome {
            verdict: Verdict::Unverifiable,
            passage: None,
            run_id: None,
            access: entry.license.clone(),
            detail: format!("passage retrieval failed: {error}"),
        },
    )
}

fn write_adjudication_request(
    context: &RecordAudit<'_>,
    input: &AnchorInput<'_>,
    passage: &Passage,
) -> anyhow::Result<(String, String)> {
    let mut request = json!({
        "audit_version": 1,
        "record_id": context.record.record_id(),
        "record_path": context.record.path.display().to_string().replace('\\', "/"),
        "claim_path": input.claim.path.as_str(),
        "claim_text": input.claim.text.as_str(),
        "source_id": input.source_id,
        "locator": input.locator,
        "passage_sha256": passage.sha256.as_str(),
        "passage_text": passage.text.as_str(),
    });
    let request_json = serde_json::to_string(&request)?;
    let request_sha256 = sha256_text(&request_json);
    // The response copies this value back. It binds the response to the
    // semantic payload above; the field itself is added only after hashing so
    // the binding cannot be circular.
    request["request_sha256"] = json!(request_sha256);
    let key = sha256_text(&format!("{}|{request_sha256}", context.record.record_id()));
    write_request(&context.options.fulfillment_dir, &key, &request)?;
    Ok((key, request_sha256))
}

fn adjudicate(
    context: &RecordAudit<'_>,
    key: &str,
    request_sha256: &str,
    passage_text: &str,
) -> (Verdict, Option<String>, String) {
    let response_path = context
        .options
        .fulfillment_dir
        .join(format!("{key}.audit.response.json"));
    if response_path.exists() {
        return response_verdict(
            &context.options.fulfillment_dir,
            key,
            request_sha256,
            context.drafting_run_id.as_deref(),
        );
    }
    let Some(gold) = &context.gold else {
        return response_verdict(
            &context.options.fulfillment_dir,
            key,
            request_sha256,
            context.drafting_run_id.as_deref(),
        );
    };
    if evidence_matches_passage(&gold.evidence, passage_text) {
        return (gold.verdict, Some(gold.run_id.clone()), gold.detail.clone());
    }
    (
        Verdict::Unverifiable,
        Some(gold.run_id.clone()),
        "gold review evidence does not match the retrieved passage".to_owned(),
    )
}

fn audit_anchor(
    context: &RecordAudit<'_>,
    input: &AnchorInput<'_>,
) -> anyhow::Result<AnchorVerdict> {
    let Some(entry) = input.access else {
        return Ok(no_access_verdict(context, input));
    };
    let passage = match retrieve_passage(
        entry,
        input.locator,
        &context.options.cache_dir,
        context.options.offline,
    ) {
        Ok(passage) => passage,
        Err(error) => return Ok(retrieval_error_verdict(context, input, entry, error)),
    };
    let (key, request_sha256) = write_adjudication_request(context, input, &passage)?;
    let (verdict, run_id, detail) = adjudicate(context, &key, &request_sha256, &passage.text);
    Ok(final_verdict(
        context,
        input,
        AnchorOutcome {
            verdict,
            passage: Some(PassageEvidence {
                sha256: passage.sha256,
                url: passage.url,
            }),
            run_id,
            access: entry.license.clone(),
            detail,
        },
    ))
}

fn audit_claim(
    context: &RecordAudit<'_>,
    claim: &ClaimBlock,
    state: &mut AuditState,
) -> anyhow::Result<()> {
    state.claim_count += 1;
    for (source_id, locator) in &claim.anchors {
        let access_entry = context.access.get(source_id);
        if access_entry.is_none() {
            state.receipt_sources.insert(source_id.clone());
        }
        let input = AnchorInput {
            claim,
            source_id,
            locator,
            access: access_entry,
        };
        state.anchors.push(audit_anchor(context, &input)?);
    }
    Ok(())
}

fn audit_record(
    run: &RunContext<'_>,
    record: &RecordFile,
    state: &mut AuditState,
) -> anyhow::Result<()> {
    let context = RecordAudit {
        record,
        options: run.options,
        access: run.access,
        gold: gold_adjudication(run.review, record),
        drafting_run_id: run
            .options
            .drafting_run_id
            .clone()
            .or_else(|| run.review.map(|review| review.drafting_run_id().to_owned())),
    };
    for claim in claim_blocks_for(record.format, &record.value) {
        audit_claim(&context, &claim, state)?;
    }
    Ok(())
}

fn validate_working_dirs(options: &AuditOptions) -> anyhow::Result<()> {
    if options.cache_dir.starts_with(options.root.join("data")) {
        bail!("citation-audit cache must be outside data/");
    }
    if options
        .fulfillment_dir
        .starts_with(options.root.join("data"))
    {
        bail!("citation-audit fulfillment files must be outside data/");
    }
    Ok(())
}

fn verdict_count(anchors: &[AnchorVerdict], verdict: Verdict) -> usize {
    anchors.iter().filter(|a| a.verdict == verdict).count()
}

fn build_report(records: usize, state: AuditState) -> AuditReport {
    let release_blocking_record_ids = state
        .anchors
        .iter()
        .filter(|a| a.verdict == Verdict::Unsupported)
        .map(|a| a.record_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let counts = AuditCounts {
        records,
        claim_blocks: state.claim_count,
        anchors: state.anchors.len(),
        supported: verdict_count(&state.anchors, Verdict::Supported),
        unsupported: verdict_count(&state.anchors, Verdict::Unsupported),
        unverifiable: verdict_count(&state.anchors, Verdict::Unverifiable),
    };
    AuditReport {
        audit_version: 1,
        counts,
        release_blocking_record_ids,
        unverifiable_by_design_sources: state.receipt_sources.into_iter().collect(),
        anchors: state.anchors,
    }
}

/// Run the audit and return the report. Writing the report is the caller's
/// responsibility so tests can inspect the exact structure first.
pub fn run_audit(options: &AuditOptions) -> anyhow::Result<AuditReport> {
    validate_working_dirs(options)?;
    let access = load_access(&options.access)?;
    let review = load_gold_review(options.gold_review.as_ref())?;
    let records = load_records(&options.root, &options.records)?;
    let run = RunContext {
        options,
        access: &access,
        review: review.as_ref(),
    };
    let mut state = AuditState::default();
    for record in &records {
        audit_record(&run, record, &mut state)?;
    }
    Ok(build_report(records.len(), state))
}

/// Serialize one report without passage text.
pub fn report_json(report: &AuditReport) -> anyhow::Result<String> {
    Ok(format!("{}\n", serde_json::to_string_pretty(report)?))
}
