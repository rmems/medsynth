use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use super::common::{collect_anchors, prompt_text, sha256_text};
use super::model::{
    AuditRecord, Disposition, DispositionFile, ReferenceItem, ReferenceLine, SplitFile,
};
use crate::originality::shingles;
use crate::{Format, RecordFile, json_files, load_paths_with_format};

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn enumeration_error(input: &Path, findings: &[crate::Finding]) -> String {
    format!(
        "cannot enumerate {}: {}",
        input.display(),
        findings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    )
}

fn expand_input(input: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if input.is_file() {
        return Ok(vec![canonical(input)]);
    }
    if !input.is_dir() {
        bail!("record input {} does not exist", input.display());
    }
    let (found, findings) = json_files(input);
    if !findings.is_empty() {
        bail!(enumeration_error(input, &findings));
    }
    Ok(found.iter().map(|path| canonical(path)).collect())
}

fn audit_format_of(path: &Path) -> Option<Format> {
    let directory_format = path.components().find_map(|part| {
        Format::all()
            .into_iter()
            .find(|format| part.as_os_str() == format.slug())
    });
    directory_format.or_else(|| {
        if path.extension().is_none_or(|e| e != "jsonl") {
            return None;
        }
        Format::all()
            .into_iter()
            .find(|format| path.file_stem().and_then(|stem| stem.to_str()) == Some(format.slug()))
    })
}

fn is_excluded_metadata(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str() == "near-miss")
        || path.file_name() == Some(std::ffi::OsStr::new("review-manifest.json"))
}

fn is_record_shard(path: &Path) -> bool {
    audit_format_of(path).is_some() && !is_excluded_metadata(path)
}

pub(crate) fn collect_record_paths(inputs: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for input in inputs {
        let expanded = expand_input(input)?;
        if input.is_file() && expanded.iter().any(|path| !is_record_shard(path)) {
            bail!(
                "explicit record input {} is not a recognizable format shard",
                input.display()
            );
        }
        paths.extend(expanded);
    }
    paths.retain(|path| is_record_shard(path));
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        bail!("no record shards found in the audit inputs");
    }
    Ok(paths)
}

fn load_error(findings: &[crate::Finding]) -> String {
    format!(
        "cannot load audit records: {}",
        findings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    )
}

fn metadata_string(record: &RecordFile, key: &str) -> String {
    record
        .value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("<missing>")
        .to_owned()
}

fn audit_record(record: RecordFile) -> anyhow::Result<AuditRecord> {
    let domain = metadata_string(&record, "domain");
    let tier = metadata_string(&record, "tier");
    let cell = format!("{}:{}:{}", domain, record.format.slug(), tier);
    let Some(prompt) = prompt_text(&record.value, record.format) else {
        bail!(
            "record `{}` has no {} prompt text to screen",
            record.record_id(),
            record.format.slug()
        );
    };
    if prompt.trim().is_empty() {
        bail!(
            "record `{}` has an empty {} prompt to screen",
            record.record_id(),
            record.format.slug()
        );
    }
    let mut anchors = BTreeSet::new();
    collect_anchors(&record.value, &mut anchors);
    let full_sha256 = sha256_text(&record.raw);
    Ok(AuditRecord {
        prompt_shingles: shingles(&prompt),
        prompt_sha256: sha256_text(&prompt),
        full_sha256,
        cell,
        anchors,
        record,
    })
}

pub(crate) fn load_audit_records(
    root: &Path,
    inputs: &[PathBuf],
) -> anyhow::Result<Vec<AuditRecord>> {
    let paths = collect_record_paths(inputs)?;
    let root = root.to_path_buf();
    let (records, findings) = load_paths_with_format(
        paths,
        move |p| p.strip_prefix(&root).unwrap_or(p).to_path_buf(),
        audit_format_of,
    );
    if !findings.is_empty() {
        bail!(load_error(&findings));
    }
    let mut out = Vec::new();
    let mut ids = BTreeSet::new();
    for record in records {
        let id = record.record_id();
        if !ids.insert(id.clone()) {
            bail!("duplicate record id `{id}` in audit inputs");
        }
        out.push(audit_record(record)?);
    }
    Ok(out)
}

fn validate_reference_fields(parsed: &ReferenceLine, line_no: usize) -> anyhow::Result<()> {
    for (field, value) in [
        ("id", parsed.id.as_str()),
        ("kind", parsed.kind.as_str()),
        ("text", parsed.text.as_str()),
        ("source_url", parsed.source_url.as_str()),
        ("license", parsed.license.as_str()),
        ("provenance", parsed.provenance.as_str()),
    ] {
        if value.trim().is_empty() {
            bail!(
                "reference `{}` at line {line_no} has empty {field}",
                parsed.id
            );
        }
    }
    Ok(())
}

fn validate_reference_license(parsed: &ReferenceLine) -> anyhow::Result<()> {
    if matches!(
        parsed.license.as_str(),
        "CC0-1.0" | "CC BY 4.0" | "CC BY-SA 4.0" | "public-domain"
    ) {
        return Ok(());
    }
    bail!(
        "reference `{}` has non-allowlisted license `{}`",
        parsed.id,
        parsed.license
    )
}

fn parse_reference(
    line: &str,
    path: &Path,
    line_no: usize,
) -> anyhow::Result<Option<ReferenceLine>> {
    if line.trim().is_empty() {
        return Ok(None);
    }
    let parsed: ReferenceLine = serde_json::from_str(line)
        .with_context(|| format!("invalid reference JSON at {}:{line_no}", path.display()))?;
    validate_reference_fields(&parsed, line_no)?;
    validate_reference_license(&parsed)?;
    Ok(Some(parsed))
}

fn reference_item(parsed: ReferenceLine, line_no: usize) -> ReferenceItem {
    ReferenceItem {
        text_sha256: sha256_text(&parsed.text),
        shingles: shingles(&parsed.text),
        id: parsed.id,
        kind: parsed.kind,
        source_url: parsed.source_url,
        license: parsed.license,
        provenance: parsed.provenance,
        line: line_no,
    }
}

fn push_unique_reference(
    items: &mut Vec<ReferenceItem>,
    ids: &mut BTreeSet<String>,
    parsed: ReferenceLine,
    line_no: usize,
) -> anyhow::Result<()> {
    if !ids.insert(parsed.id.clone()) {
        bail!("duplicate reference id `{}`", parsed.id);
    }
    items.push(reference_item(parsed, line_no));
    Ok(())
}

pub(crate) fn load_references(path: &Path) -> anyhow::Result<(Vec<ReferenceItem>, String)> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("cannot read reference corpus {}", path.display()))?;
    let file_hash = sha256_text(&text);
    let mut items = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        if let Some(parsed) = parse_reference(line, path, line_no)? {
            push_unique_reference(&mut items, &mut ids, parsed, line_no)?;
        }
    }
    if items.is_empty() {
        bail!(
            "reference corpus {} contains no items; external overlap cannot be audited",
            path.display()
        );
    }
    Ok((items, file_hash))
}

pub(crate) fn load_splits(
    path: Option<&PathBuf>,
) -> anyhow::Result<(BTreeMap<String, String>, Option<String>)> {
    let Some(path) = path else {
        return Ok((BTreeMap::new(), None));
    };
    let text = fs::read_to_string(path)
        .with_context(|| format!("cannot read split assignments {}", path.display()))?;
    let parsed: SplitFile = serde_json::from_str(&text)
        .with_context(|| format!("invalid split assignments {}", path.display()))?;
    for (id, split) in &parsed.assignments {
        if id.trim().is_empty() || split.trim().is_empty() {
            bail!("split assignments contain an empty record id or split");
        }
    }
    Ok((parsed.assignments, Some(sha256_text(&text))))
}

fn validate_disposition_fields(disposition: &Disposition) -> anyhow::Result<()> {
    let incomplete = [
        disposition.flag_id.trim(),
        disposition.reviewer_run_id.trim(),
        disposition.rationale.trim(),
    ]
    .iter()
    .any(|value| value.is_empty());
    if incomplete {
        bail!("every disposition needs flag_id, reviewer_run_id, and rationale");
    }
    Ok(())
}

fn validate_disposition_decision(disposition: &Disposition) -> anyhow::Result<()> {
    if matches!(
        disposition.decision.as_str(),
        "rewrite" | "replace" | "not-a-duplicate"
    ) {
        return Ok(());
    }
    bail!(
        "disposition `{}` has invalid decision `{}`",
        disposition.flag_id,
        disposition.decision
    )
}

fn insert_disposition(
    out: &mut HashMap<String, Disposition>,
    disposition: Disposition,
) -> anyhow::Result<()> {
    validate_disposition_fields(&disposition)?;
    validate_disposition_decision(&disposition)?;
    if out
        .insert(disposition.flag_id.clone(), disposition)
        .is_some()
    {
        bail!("duplicate disposition flag id");
    }
    Ok(())
}

pub(crate) fn load_dispositions(
    path: Option<&PathBuf>,
) -> anyhow::Result<(HashMap<String, Disposition>, Option<String>)> {
    let Some(path) = path else {
        return Ok((HashMap::new(), None));
    };
    let text = fs::read_to_string(path)
        .with_context(|| format!("cannot read dispositions {}", path.display()))?;
    let parsed: DispositionFile = serde_json::from_str(&text)
        .with_context(|| format!("invalid dispositions {}", path.display()))?;
    let mut out = HashMap::new();
    for disposition in parsed.dispositions {
        insert_disposition(&mut out, disposition)?;
    }
    Ok((out, Some(sha256_text(&text))))
}
