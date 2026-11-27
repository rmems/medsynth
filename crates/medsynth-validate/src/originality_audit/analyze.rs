use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use super::common::{display_path, sha256_text};
use super::model::{
    AuditFlag, AuditRecord, ContaminationGroup, Disposition, RecordSummary, ReferenceItem,
    ReferenceSummary,
};
use crate::originality::containment;

pub(crate) fn flag_id(kind: &str, ids: &[String], reference: Option<&str>) -> String {
    let material = format!("{kind}|{}|{}", ids.join("|"), reference.unwrap_or("-"));
    format!("flag-{}", &sha256_text(&material)[..16])
}

pub(crate) fn overlap_keys(a: &BTreeSet<String>, b: &BTreeSet<String>) -> Vec<String> {
    a.intersection(b).cloned().collect()
}

pub(crate) fn rounded(value: f64) -> f64 {
    (value * 10000.0).round() / 10000.0
}

struct DisjointSet {
    parent: Vec<usize>,
}

impl DisjointSet {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
        }
    }

    fn find(&mut self, index: usize) -> usize {
        if self.parent[index] != index {
            let root = self.find(self.parent[index]);
            self.parent[index] = root;
        }
        self.parent[index]
    }

    fn union(&mut self, a: usize, b: usize) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent[rb] = ra;
        }
    }
}

pub(crate) fn apply_dispositions(
    mut flags: Vec<AuditFlag>,
    dispositions: &HashMap<String, Disposition>,
) -> (Vec<AuditFlag>, Vec<String>, usize) {
    let mut used = HashSet::new();
    let mut applied = 0;
    for flag in &mut flags {
        if let Some(disposition) = dispositions.get(&flag.flag_id) {
            used.insert(flag.flag_id.clone());
            applied += 1;
            flag.status = if disposition.decision == "not-a-duplicate" {
                flag.release_blocking = false;
                "reviewed-not-a-duplicate".to_owned()
            } else {
                flag.release_blocking = true;
                format!("reviewed-{}", disposition.decision)
            };
            flag.disposition = Some(disposition.clone());
        }
    }
    let mut unused: Vec<String> = dispositions
        .keys()
        .filter(|id| !used.contains(*id))
        .cloned()
        .collect();
    unused.sort();
    (flags, unused, applied)
}

pub(crate) fn external_overlap_flags(
    records: &[AuditRecord],
    references: &[ReferenceItem],
    threshold: f64,
) -> Vec<AuditFlag> {
    let mut flags = Vec::new();
    for record in records {
        for reference in references {
            let score = containment(&record.prompt_shingles, &reference.shingles);
            if score < threshold {
                continue;
            }
            let ids = vec![record.record.record_id()];
            flags.push(AuditFlag {
                flag_id: flag_id("external-overlap", &ids, Some(&reference.id)),
                kind: "external-overlap".to_owned(),
                record_ids: ids,
                reference_id: Some(reference.id.clone()),
                coverage_cell: Some(record.cell.clone()),
                anchor_passage_overlap: Vec::new(),
                containment: rounded(score),
                threshold,
                status: "needs-review".to_owned(),
                disposition: None,
                release_blocking: true,
            });
        }
    }
    flags
}

pub(crate) fn internal_dedup_flag(
    a: &AuditRecord,
    b: &AuditRecord,
    shared: Vec<String>,
    score: f64,
    threshold: f64,
) -> AuditFlag {
    let mut ids = vec![a.record.record_id(), b.record.record_id()];
    ids.sort();
    AuditFlag {
        flag_id: flag_id("internal-dedup", &ids, None),
        kind: "internal-dedup".to_owned(),
        record_ids: ids,
        reference_id: None,
        coverage_cell: Some(a.cell.clone()),
        anchor_passage_overlap: shared,
        containment: rounded(score),
        threshold,
        status: "needs-review".to_owned(),
        disposition: None,
        release_blocking: true,
    }
}

pub(crate) fn group_members(
    records: &[AuditRecord],
    members_index: &[usize],
    pair_scores: &HashMap<(usize, usize), f64>,
    splits: &BTreeMap<String, String>,
) -> ContaminationGroup {
    let mut ids: Vec<String> = members_index
        .iter()
        .map(|i| records[*i].record.record_id())
        .collect();
    ids.sort();
    let mut shared_intersection: Option<BTreeSet<String>> = None;
    for i in members_index {
        let anchors = records[*i].anchors.clone();
        shared_intersection = Some(match shared_intersection {
            None => anchors,
            Some(existing) => existing.intersection(&anchors).cloned().collect(),
        });
    }
    let mut max_score: f64 = 0.0;
    for x in 0..members_index.len() {
        for y in (x + 1)..members_index.len() {
            let aa = members_index[x];
            let bb = members_index[y];
            let key = (aa.min(bb), aa.max(bb));
            max_score = max_score.max(pair_scores.get(&key).copied().unwrap_or(0.0));
        }
    }
    let assignments = ids
        .iter()
        .filter_map(|id| splits.get(id).map(|split| (id.clone(), split.clone())))
        .collect();
    ContaminationGroup {
        group_id: format!("group-{}", &sha256_text(&ids.join("|"))[..12]),
        record_ids: ids,
        shared_anchor_passages: shared_intersection
            .unwrap_or_default()
            .into_iter()
            .collect(),
        max_containment: rounded(max_score),
        split_assignments: assignments,
    }
}

pub(crate) struct InternalScan {
    pub(crate) flags: Vec<AuditFlag>,
    pub(crate) groups: Vec<ContaminationGroup>,
    pub(crate) pairs: u64,
}

pub(crate) fn scan_internal(
    records: &[AuditRecord],
    dedup_threshold: f64,
    contamination_threshold: f64,
    splits: &BTreeMap<String, String>,
) -> InternalScan {
    let pairs = (records.len() as u64 * records.len().saturating_sub(1) as u64) / 2;
    let mut flags = Vec::new();
    let mut disjoint = DisjointSet::new(records.len());
    let mut pair_scores = HashMap::new();
    for i in 0..records.len() {
        for j in (i + 1)..records.len() {
            let a = &records[i];
            let b = &records[j];
            let score = containment(&a.prompt_shingles, &b.prompt_shingles);
            let shared = overlap_keys(&a.anchors, &b.anchors);
            if score >= contamination_threshold && !shared.is_empty() {
                disjoint.union(i, j);
                pair_scores.insert((i, j), score);
            }
            if a.cell == b.cell && !shared.is_empty() && score >= dedup_threshold {
                flags.push(internal_dedup_flag(a, b, shared, score, dedup_threshold));
            }
        }
    }

    let mut grouped: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..records.len() {
        let root_index = disjoint.find(index);
        grouped.entry(root_index).or_default().push(index);
    }
    let mut groups: Vec<ContaminationGroup> = grouped
        .values()
        .filter(|members| members.len() > 1)
        .map(|members| group_members(records, members, &pair_scores, splits))
        .collect();
    groups.sort_by(|a, b| a.group_id.cmp(&b.group_id));
    InternalScan {
        flags,
        groups,
        pairs,
    }
}

pub(crate) fn cross_split_flags(groups: &[ContaminationGroup], threshold: f64) -> Vec<AuditFlag> {
    groups
        .iter()
        .filter(|group| {
            group
                .split_assignments
                .values()
                .collect::<BTreeSet<_>>()
                .len()
                > 1
        })
        .map(|group| {
            let mut ids = group.record_ids.clone();
            ids.sort();
            AuditFlag {
                flag_id: flag_id("cross-split-contamination", &ids, None),
                kind: "cross-split-contamination".to_owned(),
                record_ids: ids,
                reference_id: None,
                coverage_cell: None,
                anchor_passage_overlap: group.shared_anchor_passages.clone(),
                containment: group.max_containment,
                threshold,
                status: "needs-review".to_owned(),
                disposition: None,
                release_blocking: true,
            }
        })
        .collect()
}

pub(crate) fn record_summaries(root: &Path, records: &[AuditRecord]) -> Vec<RecordSummary> {
    records
        .iter()
        .map(|record| RecordSummary {
            record_id: record.record.record_id(),
            path: display_path(root, &record.record.path),
            line: record.record.line,
            format: record.record.format.slug().to_owned(),
            domain: record
                .record
                .value
                .get("domain")
                .and_then(|v| v.as_str())
                .unwrap_or("<missing>")
                .to_owned(),
            tier: record
                .record
                .value
                .get("tier")
                .and_then(|v| v.as_str())
                .unwrap_or("<missing>")
                .to_owned(),
            coverage_cell: record.cell.clone(),
            prompt_sha256: record.prompt_sha256.clone(),
            record_text_sha256: record.full_sha256.clone(),
            anchor_passages: record.anchors.iter().cloned().collect(),
        })
        .collect()
}

pub(crate) fn reference_summaries(references: &[ReferenceItem]) -> Vec<ReferenceSummary> {
    references
        .iter()
        .map(|reference| ReferenceSummary {
            reference_id: reference.id.clone(),
            kind: reference.kind.clone(),
            source_url: reference.source_url.clone(),
            license: reference.license.clone(),
            provenance: reference.provenance.clone(),
            text_sha256: reference.text_sha256.clone(),
            line: reference.line,
        })
        .collect()
}

pub(crate) fn corpus_hash(root: &Path, records: &[AuditRecord]) -> String {
    let material = records
        .iter()
        .map(|record| {
            format!(
                "{}:{}:{}",
                display_path(root, &record.record.path),
                record.record.line.unwrap_or(0),
                record.full_sha256
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    sha256_text(&material)
}

pub(crate) fn count_kind(flags: &[AuditFlag], kind: &str) -> usize {
    flags.iter().filter(|flag| flag.kind == kind).count()
}
