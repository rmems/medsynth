use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use super::common::{collect_anchors, prompt_text, sha256_text};
use super::model::{
    AuditRecord, Disposition, DispositionFile, ReferenceItem, ReferenceLine, SplitFile,
};
use crate::originality::{record_text, shingles};
use crate::{Format, json_files, load_paths};

pub(crate) fn collect_record_paths(inputs: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for input in inputs {
        if input.is_file() {
            paths.push(input.canonicalize().unwrap_or_else(|_| input.clone()));
        } else if input.is_dir() {
            let (found, findings) = json_files(input);
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
            paths.extend(
                found
                    .into_iter()
                    .map(|path| path.canonicalize().unwrap_or(path)),
            );
        } else {
            bail!("record input {} does not exist", input.display());
        }
    }
    paths.retain(|path| {
        let format_component = path.components().any(|part| {
            Format::all()
                .into_iter()
                .any(|format| part.as_os_str() == format.slug())
        });
        let format_stem = Format::all()
            .into_iter()
            .any(|format| path.file_stem().and_then(|stem| stem.to_str()) == Some(format.slug()));
        (format_component || format_stem)
            && !path
                .components()
                .any(|part| part.as_os_str() == "near-miss")
            && path.file_name() != Some(std::ffi::OsStr::new("review-manifest.json"))
    });
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        bail!("no record shards found in the audit inputs");
    }
    Ok(paths)
}

pub(crate) fn load_audit_records(
    root: &Path,
    inputs: &[PathBuf],
) -> anyhow::Result<Vec<AuditRecord>> {
    let paths = collect_record_paths(inputs)?;
    let root = root.to_path_buf();
    let (records, findings) = load_paths(paths, move |p| {
        p.strip_prefix(&root).unwrap_or(p).to_path_buf()
    });
    if !findings.is_empty() {
        bail!(
            "cannot load audit records: {}",
            findings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    let mut out = Vec::new();
    let mut ids = BTreeSet::new();
    for record in records {
        let id = record.record_id();
        if !ids.insert(id.clone()) {
            bail!("duplicate record id `{id}` in audit inputs");
        }
        let domain = record
            .value
            .get("domain")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing>")
            .to_owned();
        let tier = record
            .value
            .get("tier")
            .and_then(|v| v.as_str())
            .unwrap_or("<missing>")
            .to_owned();
        let cell = format!("{}:{}:{}", domain, record.format.slug(), tier);
        let prompt = prompt_text(&record.value, record.format);
        let mut anchors = BTreeSet::new();
        collect_anchors(&record.value, &mut anchors);
        out.push(AuditRecord {
            prompt_shingles: shingles(&prompt),
            prompt_sha256: sha256_text(&prompt),
            full_sha256: sha256_text(&record_text(&record.value)),
            cell,
            anchors,
            record,
        });
    }
    Ok(out)
}

pub(crate) fn load_references(path: &Path) -> anyhow::Result<(Vec<ReferenceItem>, String)> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("cannot read reference corpus {}", path.display()))?;
    let file_hash = sha256_text(&text);
    let mut items = Vec::new();
    let mut ids = BTreeSet::new();
    for (index, line) in text.lines().enumerate() {
        let line_no = index + 1;
        if line.trim().is_empty() {
            continue;
        }
        let parsed: ReferenceLine = serde_json::from_str(line)
            .with_context(|| format!("invalid reference JSON at {}:{line_no}", path.display()))?;
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
        if !matches!(
            parsed.license.as_str(),
            "CC0-1.0" | "CC BY 4.0" | "CC BY-SA 4.0" | "public-domain"
        ) {
            bail!(
                "reference `{}` has non-allowlisted license `{}`",
                parsed.id,
                parsed.license
            );
        }
        if !ids.insert(parsed.id.clone()) {
            bail!("duplicate reference id `{}`", parsed.id);
        }
        items.push(ReferenceItem {
            text_sha256: sha256_text(&parsed.text),
            shingles: shingles(&parsed.text),
            id: parsed.id,
            kind: parsed.kind,
            source_url: parsed.source_url,
            license: parsed.license,
            provenance: parsed.provenance,
            line: line_no,
        });
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
        if disposition.flag_id.trim().is_empty()
            || disposition.reviewer_run_id.trim().is_empty()
            || disposition.rationale.trim().is_empty()
        {
            bail!("every disposition needs flag_id, reviewer_run_id, and rationale");
        }
        if !matches!(
            disposition.decision.as_str(),
            "rewrite" | "replace" | "not-a-duplicate"
        ) {
            bail!(
                "disposition `{}` has invalid decision `{}`",
                disposition.flag_id,
                disposition.decision
            );
        }
        if out
            .insert(disposition.flag_id.clone(), disposition)
            .is_some()
        {
            bail!("duplicate disposition flag id");
        }
    }
    Ok((out, Some(sha256_text(&text))))
}
