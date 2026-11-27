use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

use super::common::{display_path, sha256_text};
use super::model::{
    AuditFlag, AuditRecord, ContaminationGroup, ContaminationLink, Disposition, RecordSummary,
    ReferenceItem, ReferenceSummary,
};
use crate::originality::containment;

pub(crate) fn flag_id(
    kind: &str,
    ids: &[String],
    reference: Option<&str>,
    evidence: &str,
) -> String {
    let material = format!(
        "{kind}|{}|{}|{}",
        ids.join("|"),
        reference.unwrap_or("-"),
        evidence
    );
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

fn reviewed_status(flag: &mut AuditFlag, decision: &str) {
    if decision == "not-a-duplicate" {
        flag.release_blocking = false;
        flag.status = "reviewed-not-a-duplicate".to_owned();
    } else {
        flag.release_blocking = true;
        flag.status = format!("reviewed-{decision}");
    }
}

fn apply_one_disposition(
    flag: &mut AuditFlag,
    dispositions: &HashMap<String, Disposition>,
    used: &mut HashSet<String>,
) -> bool {
    let Some(disposition) = dispositions.get(&flag.flag_id) else {
        return false;
    };
    used.insert(flag.flag_id.clone());
    reviewed_status(flag, &disposition.decision);
    flag.disposition = Some(disposition.clone());
    true
}

pub(crate) fn apply_dispositions(
    mut flags: Vec<AuditFlag>,
    dispositions: &HashMap<String, Disposition>,
) -> (Vec<AuditFlag>, Vec<String>, usize) {
    let mut used = HashSet::new();
    let mut applied = 0;
    for flag in &mut flags {
        if apply_one_disposition(flag, dispositions, &mut used) {
            applied += 1;
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

fn external_overlap_flag(
    record: &AuditRecord,
    reference: &ReferenceItem,
    score: f64,
    threshold: f64,
) -> AuditFlag {
    let ids = vec![record.record.record_id()];
    AuditFlag {
        flag_id: flag_id(
            "external-overlap",
            &ids,
            Some(&reference.id),
            &format!("{}:{}", record.full_sha256, reference.text_sha256),
        ),
        kind: "external-overlap".to_owned(),
        record_ids: ids,
        reference_id: Some(reference.id.clone()),
        coverage_cell: Some(record.cell.clone()),
        anchor_passage_overlap: Vec::new(),
        evidence_links: Vec::new(),
        containment: rounded(score),
        threshold,
        status: "needs-review".to_owned(),
        disposition: None,
        release_blocking: true,
    }
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
            if score >= threshold {
                flags.push(external_overlap_flag(record, reference, score, threshold));
            }
        }
    }
    flags
}

pub(crate) struct PairEvidence {
    shared: Vec<String>,
    score: f64,
}

fn internal_evidence(a: &AuditRecord, b: &AuditRecord) -> String {
    let mut members = [
        (a.record.record_id(), a.full_sha256.clone()),
        (b.record.record_id(), b.full_sha256.clone()),
    ];
    members.sort_by(|left, right| left.0.cmp(&right.0));
    members
        .iter()
        .map(|(id, hash)| format!("{id}:{hash}"))
        .collect::<Vec<_>>()
        .join("|")
}

pub(crate) fn internal_dedup_flag(
    a: &AuditRecord,
    b: &AuditRecord,
    evidence: &PairEvidence,
    threshold: f64,
) -> AuditFlag {
    let mut ids = vec![a.record.record_id(), b.record.record_id()];
    ids.sort();
    AuditFlag {
        flag_id: flag_id("internal-dedup", &ids, None, &internal_evidence(a, b)),
        kind: "internal-dedup".to_owned(),
        record_ids: ids,
        reference_id: None,
        coverage_cell: Some(a.cell.clone()),
        anchor_passage_overlap: evidence.shared.clone(),
        evidence_links: Vec::new(),
        containment: rounded(evidence.score),
        threshold,
        status: "needs-review".to_owned(),
        disposition: None,
        release_blocking: true,
    }
}

fn shared_anchor_intersection(
    records: &[AuditRecord],
    members_index: &[usize],
) -> BTreeSet<String> {
    let mut intersection: Option<BTreeSet<String>> = None;
    for index in members_index {
        let anchors = records[*index].anchors.clone();
        intersection = Some(match intersection {
            None => anchors,
            Some(existing) => existing.intersection(&anchors).cloned().collect(),
        });
    }
    intersection.unwrap_or_default()
}

fn pair_key(a: usize, b: usize) -> (usize, usize) {
    (a.min(b), a.max(b))
}

fn max_pair_score(members_index: &[usize], pair_scores: &HashMap<(usize, usize), f64>) -> f64 {
    let mut max_score: f64 = 0.0;
    for x in 0..members_index.len() {
        for y in (x + 1)..members_index.len() {
            let key = pair_key(members_index[x], members_index[y]);
            max_score = max_score.max(pair_scores.get(&key).copied().unwrap_or(0.0));
        }
    }
    max_score
}

fn contamination_links(
    records: &[AuditRecord],
    members_index: &[usize],
    pair_scores: &HashMap<(usize, usize), f64>,
    splits: &BTreeMap<String, String>,
) -> Vec<ContaminationLink> {
    let mut links = Vec::new();
    for x in 0..members_index.len() {
        for y in (x + 1)..members_index.len() {
            let left = members_index[x];
            let right = members_index[y];
            let Some(score) = pair_scores.get(&pair_key(left, right)) else {
                continue;
            };
            let a = &records[left];
            let b = &records[right];
            let mut ids = vec![a.record.record_id(), b.record.record_id()];
            ids.sort();
            let assignments = ids
                .iter()
                .filter_map(|id| splits.get(id).map(|split| (id.clone(), split.clone())))
                .collect();
            links.push(ContaminationLink {
                record_ids: ids,
                shared_anchor_passages: overlap_keys(&a.anchors, &b.anchors),
                containment: rounded(*score),
                split_assignments: assignments,
            });
        }
    }
    links
}

fn record_hashes(records: &[AuditRecord], members_index: &[usize]) -> BTreeMap<String, String> {
    members_index
        .iter()
        .map(|index| {
            let record = &records[*index];
            (record.record.record_id(), record.full_sha256.clone())
        })
        .collect()
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
    let assignments = ids
        .iter()
        .filter_map(|id| splits.get(id).map(|split| (id.clone(), split.clone())))
        .collect();
    ContaminationGroup {
        group_id: format!("group-{}", &sha256_text(&ids.join("|"))[..12]),
        record_ids: ids,
        shared_anchor_passages: shared_anchor_intersection(records, members_index)
            .into_iter()
            .collect(),
        evidence_links: contamination_links(records, members_index, pair_scores, splits),
        record_text_sha256: record_hashes(records, members_index),
        max_containment: rounded(max_pair_score(members_index, pair_scores)),
        split_assignments: assignments,
    }
}

pub(crate) struct InternalScan {
    pub(crate) flags: Vec<AuditFlag>,
    pub(crate) groups: Vec<ContaminationGroup>,
    pub(crate) pairs: u64,
}

fn contamination_pair(evidence: &PairEvidence, threshold: f64) -> bool {
    evidence.score >= threshold && !evidence.shared.is_empty()
}

fn same_cell_duplicate(
    a: &AuditRecord,
    b: &AuditRecord,
    evidence: &PairEvidence,
    threshold: f64,
) -> bool {
    a.cell == b.cell && !evidence.shared.is_empty() && evidence.score >= threshold
}

struct PairScanner<'a> {
    records: &'a [AuditRecord],
    dedup_threshold: f64,
    contamination_threshold: f64,
    flags: Vec<AuditFlag>,
    disjoint: DisjointSet,
    pair_scores: HashMap<(usize, usize), f64>,
}

impl<'a> PairScanner<'a> {
    fn new(records: &'a [AuditRecord], dedup_threshold: f64, contamination_threshold: f64) -> Self {
        Self {
            records,
            dedup_threshold,
            contamination_threshold,
            flags: Vec::new(),
            disjoint: DisjointSet::new(records.len()),
            pair_scores: HashMap::new(),
        }
    }

    fn scan_pair(&mut self, i: usize, j: usize) {
        let a = &self.records[i];
        let b = &self.records[j];
        let evidence = PairEvidence {
            shared: overlap_keys(&a.anchors, &b.anchors),
            score: containment(&a.prompt_shingles, &b.prompt_shingles),
        };
        if contamination_pair(&evidence, self.contamination_threshold) {
            self.disjoint.union(i, j);
            self.pair_scores.insert((i, j), evidence.score);
        }
        if same_cell_duplicate(a, b, &evidence, self.dedup_threshold) {
            self.flags
                .push(internal_dedup_flag(a, b, &evidence, self.dedup_threshold));
        }
    }

    fn scan_all(mut self) -> (Vec<AuditFlag>, DisjointSet, HashMap<(usize, usize), f64>) {
        for i in 0..self.records.len() {
            for j in (i + 1)..self.records.len() {
                self.scan_pair(i, j);
            }
        }
        (self.flags, self.disjoint, self.pair_scores)
    }
}

fn scan_pairs(
    records: &[AuditRecord],
    dedup_threshold: f64,
    contamination_threshold: f64,
) -> (Vec<AuditFlag>, DisjointSet, HashMap<(usize, usize), f64>) {
    PairScanner::new(records, dedup_threshold, contamination_threshold).scan_all()
}

fn grouped_members(records_len: usize, disjoint: &mut DisjointSet) -> BTreeMap<usize, Vec<usize>> {
    let mut grouped: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for index in 0..records_len {
        let root_index = disjoint.find(index);
        grouped.entry(root_index).or_default().push(index);
    }
    grouped
}

fn build_groups(
    records: &[AuditRecord],
    disjoint: &mut DisjointSet,
    pair_scores: &HashMap<(usize, usize), f64>,
    splits: &BTreeMap<String, String>,
) -> Vec<ContaminationGroup> {
    let grouped = grouped_members(records.len(), disjoint);
    let mut groups: Vec<ContaminationGroup> = grouped
        .values()
        .filter(|members| members.len() > 1)
        .map(|members| group_members(records, members, pair_scores, splits))
        .collect();
    groups.sort_by(|a, b| a.group_id.cmp(&b.group_id));
    groups
}

pub(crate) fn scan_internal(
    records: &[AuditRecord],
    dedup_threshold: f64,
    contamination_threshold: f64,
    splits: &BTreeMap<String, String>,
) -> InternalScan {
    let pairs = (records.len() as u64 * records.len().saturating_sub(1) as u64) / 2;
    let (flags, mut disjoint, pair_scores) =
        scan_pairs(records, dedup_threshold, contamination_threshold);
    let groups = build_groups(records, &mut disjoint, &pair_scores, splits);
    InternalScan {
        flags,
        groups,
        pairs,
    }
}

fn crosses_splits(group: &ContaminationGroup) -> bool {
    group
        .split_assignments
        .values()
        .collect::<BTreeSet<_>>()
        .len()
        > 1
}

fn cross_split_evidence(group: &ContaminationGroup) -> String {
    group
        .record_ids
        .iter()
        .map(|id| {
            format!(
                "{}:{}:{}",
                id,
                group
                    .record_text_sha256
                    .get(id)
                    .map(String::as_str)
                    .unwrap_or(""),
                group
                    .split_assignments
                    .get(id)
                    .map(String::as_str)
                    .unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn cross_split_flag(group: &ContaminationGroup, threshold: f64) -> AuditFlag {
    let mut ids = group.record_ids.clone();
    ids.sort();
    AuditFlag {
        flag_id: flag_id(
            "cross-split-contamination",
            &ids,
            None,
            &cross_split_evidence(group),
        ),
        kind: "cross-split-contamination".to_owned(),
        record_ids: ids,
        reference_id: None,
        coverage_cell: None,
        anchor_passage_overlap: group.shared_anchor_passages.clone(),
        evidence_links: group.evidence_links.clone(),
        containment: group.max_containment,
        threshold,
        status: "needs-review".to_owned(),
        disposition: None,
        release_blocking: true,
    }
}

pub(crate) fn cross_split_flags(groups: &[ContaminationGroup], threshold: f64) -> Vec<AuditFlag> {
    groups
        .iter()
        .filter(|group| crosses_splits(group))
        .map(|group| cross_split_flag(group, threshold))
        .collect()
}

fn metadata_string(record: &AuditRecord, key: &str) -> String {
    record
        .record
        .value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("<missing>")
        .to_owned()
}

fn record_summary(root: &Path, record: &AuditRecord) -> RecordSummary {
    RecordSummary {
        record_id: record.record.record_id(),
        path: display_path(root, &record.record.path),
        line: record.record.line,
        format: record.record.format.slug().to_owned(),
        domain: metadata_string(record, "domain"),
        tier: metadata_string(record, "tier"),
        coverage_cell: record.cell.clone(),
        prompt_sha256: record.prompt_sha256.clone(),
        record_text_sha256: record.full_sha256.clone(),
        anchor_passages: record.anchors.iter().cloned().collect(),
    }
}

pub(crate) fn record_summaries(root: &Path, records: &[AuditRecord]) -> Vec<RecordSummary> {
    records
        .iter()
        .map(|record| record_summary(root, record))
        .collect()
}

fn reference_summary(reference: &ReferenceItem) -> ReferenceSummary {
    ReferenceSummary {
        reference_id: reference.id.clone(),
        kind: reference.kind.clone(),
        source_url: reference.source_url.clone(),
        license: reference.license.clone(),
        provenance: reference.provenance.clone(),
        text_sha256: reference.text_sha256.clone(),
        line: reference.line,
    }
}

pub(crate) fn reference_summaries(references: &[ReferenceItem]) -> Vec<ReferenceSummary> {
    references.iter().map(reference_summary).collect()
}

fn corpus_material(root: &Path, record: &AuditRecord) -> String {
    format!(
        "{}:{}:{}",
        display_path(root, &record.record.path),
        record.record.line.unwrap_or(0),
        record.full_sha256
    )
}

pub(crate) fn corpus_hash(root: &Path, records: &[AuditRecord]) -> String {
    let material = records
        .iter()
        .map(|record| corpus_material(root, record))
        .collect::<Vec<_>>()
        .join("\n");
    sha256_text(&material)
}

pub(crate) fn count_kind(flags: &[AuditFlag], kind: &str) -> usize {
    flags.iter().filter(|flag| flag.kind == kind).count()
}
