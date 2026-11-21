/// The smoke originality screen.
///
/// Word **5-gram** shingles are built from each record's text fields — every
/// string under keys `text`, `presentation`, `question`, and every string
/// inside `findings`/`options` arrays — lowercased, alphanumeric tokens,
/// hashed with a stable FNV-1a so results are identical across toolchains
/// and platforms. A pair of records is flagged when either record's shingle
/// set is contained in the other's at a fraction ≥ **0.6**: a copied or
/// near-copied record shares essentially all of its shingles with its
/// source, while two records merely on the same topic share few 5-grams.
/// Both records of an offending pair are reported. This is the smoke
/// screen only; the full originality and contamination audit is #11.
use std::collections::{HashMap, HashSet};

use crate::{Check, Finding, RecordFile};

/// Word n-gram size for the overlap shingles.
pub const NGRAM: usize = 5;
/// Containment fraction at or above which a record pair is flagged as
/// likely duplicates.
pub const THRESHOLD: f64 = 0.6;

/// The text fields the screen reads: string values under these keys,
/// plus all string members of `findings` and `options` arrays.
const TEXT_KEYS: [&str; 3] = ["text", "presentation", "question"];
const TEXT_ARRAY_KEYS: [&str; 2] = ["findings", "options"];

/// All record text the screen sees, concatenated.
pub fn record_text(value: &serde_json::Value) -> String {
    let mut out = String::new();
    collect(value, None, &mut out);
    out
}

fn collect(value: &serde_json::Value, key: Option<&str>, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                collect(v, Some(k), out);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect(item, key, out);
            }
        }
        serde_json::Value::String(s)
            if key.is_some_and(|k| TEXT_KEYS.contains(&k))
                || key.is_some_and(|k| TEXT_ARRAY_KEYS.contains(&k)) =>
        {
            out.push_str(s);
            out.push(' ');
        }
        _ => {}
    }
}

/// FNV-1a over the token — stable across Rust versions and platforms,
/// unlike `DefaultHasher`.
fn fnv1a(token: &[u8], seed: u64) -> u64 {
    token.iter().fold(seed, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

fn shingles(text: &str) -> HashSet<u64> {
    let tokens: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    tokens
        .windows(NGRAM)
        .map(|w| fnv1a(w.join("\x1f").as_bytes(), 0xcbf2_9ce4_8422_2325))
        .collect()
}

/// Flag record pairs whose shingle containment — |shared| / |smaller
/// set| — reaches [`THRESHOLD`] in either direction. Both records of a
/// pair are reported.
pub fn screen(records: &[RecordFile], findings: &mut Vec<Finding>) {
    let sets: Vec<HashSet<u64>> = records
        .iter()
        .map(|r| shingles(&record_text(&r.value)))
        .collect();
    let mut flagged: HashMap<usize, usize> = HashMap::new(); // record -> first offender
    for i in 0..sets.len() {
        for j in (i + 1)..sets.len() {
            let (a, b) = (&sets[i], &sets[j]);
            if a.is_empty() || b.is_empty() {
                continue;
            }
            let shared = a.intersection(b).count();
            let smaller = a.len().min(b.len()) as f64;
            if shared as f64 / smaller >= THRESHOLD {
                flagged.entry(i).or_insert(j);
                flagged.entry(j).or_insert(i);
            }
        }
    }
    for (i, j) in flagged {
        findings.push(Finding {
            path: records[i].path.clone(),
            line: records[i].line,
            record_id: records[i].record_id(),
            check: Check::Originality,
            detail: format!(
                "text near-duplicates {} (5-gram containment >= {THRESHOLD})",
                records[j].path.display()
            ),
        });
    }
}
