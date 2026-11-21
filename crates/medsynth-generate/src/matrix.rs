//! The taxonomy §5 coverage matrix, read from `docs/taxonomy.md`.
//!
//! Targets are read from the document rather than duplicated in code, so a
//! taxonomy amendment changes the pipeline's target in one place. The parser
//! is strict: it requires the twelve domain × format rows, checks each row's
//! subtotal, and checks the v0.1 grand total of 300.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use medsynth_schema::{Domain, Tier};

use crate::error::GenerateError;
use crate::format::Format;
use crate::hash::{normalize_newlines, sha256_str};

/// A matrix cell: domain × format × tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CellKey {
    pub domain: Domain,
    pub format: Format,
    pub tier: Tier,
}

impl PartialOrd for CellKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CellKey {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.label().cmp(&other.label())
    }
}

impl CellKey {
    /// Stable human-readable label, e.g. `bbf/mcq/foundation`.
    pub fn label(self) -> String {
        format!(
            "{}/{}/{}",
            domain_slug(self.domain),
            self.format.slug(),
            tier_slug(self.tier)
        )
    }
}

/// The v0.1 coverage matrix.
#[derive(Debug, Clone)]
pub struct CoverageMatrix {
    pub path: PathBuf,
    pub version: String,
    pub sha256: String,
    cells: BTreeMap<CellKey, u32>,
}

const V01_TOTAL: u32 = 300;

impl CoverageMatrix {
    /// Read and validate the matrix from `docs/taxonomy.md`.
    pub fn load(path: &Path) -> Result<Self, GenerateError> {
        let text =
            std::fs::read_to_string(path).map_err(|e| GenerateError::io(path.to_path_buf(), e))?;
        let version = taxonomy_version(&text).ok_or_else(|| {
            GenerateError::Matrix(format!(
                "{}: could not find the `**Version:**` line",
                path.display()
            ))
        })?;
        let mut cells = BTreeMap::new();
        let mut in_section = false;
        for line in text.lines() {
            if line.starts_with("## 5. Coverage matrix") {
                in_section = true;
                continue;
            }
            if in_section && line.starts_with("### 5.1") {
                break;
            }
            if !in_section {
                continue;
            }
            let Some(row) = parse_matrix_row(line) else {
                continue;
            };
            if cells
                .insert(
                    CellKey {
                        domain: row.domain,
                        format: row.format,
                        tier: Tier::Foundation,
                    },
                    row.foundation,
                )
                .is_some()
            {
                return Err(GenerateError::Matrix(format!(
                    "{}: duplicate matrix row for {}",
                    path.display(),
                    row.format
                )));
            }
            cells.insert(
                CellKey {
                    domain: row.domain,
                    format: row.format,
                    tier: Tier::Application,
                },
                row.application,
            );
            cells.insert(
                CellKey {
                    domain: row.domain,
                    format: row.format,
                    tier: Tier::Reasoning,
                },
                row.reasoning,
            );
        }
        if cells.len() != 12 * 3 {
            return Err(GenerateError::Matrix(format!(
                "{}: expected 12 domain × format rows (36 cells), parsed {}",
                path.display(),
                cells.len()
            )));
        }
        let total: u32 = cells.values().sum();
        if version == "0.1" && total != V01_TOTAL {
            return Err(GenerateError::Matrix(format!(
                "{}: v0.1 matrix sums to {total}, expected {V01_TOTAL}",
                path.display()
            )));
        }
        let sha256 = sha256_str(&normalize_newlines(&text));
        Ok(Self {
            path: path.to_path_buf(),
            version,
            sha256,
            cells,
        })
    }

    pub fn target(&self, key: CellKey) -> Option<u32> {
        self.cells.get(&key).copied()
    }

    pub fn cells(&self) -> impl Iterator<Item = (CellKey, u32)> + '_ {
        self.cells.iter().map(|(k, v)| (*k, *v))
    }

    pub fn total(&self) -> u32 {
        self.cells.values().sum()
    }
}

struct MatrixRow {
    domain: Domain,
    format: Format,
    foundation: u32,
    application: u32,
    reasoning: u32,
}

fn parse_matrix_row(line: &str) -> Option<MatrixRow> {
    let cols: Vec<&str> = line.split('|').map(str::trim).collect();
    // "| `bbf` | `mcq` | 18 | 16 | 8 | 42 |" → ["", "`bbf`", "`mcq`",
    // "18", "16", "8", "42", ""]
    if cols.len() != 8 {
        return None;
    }
    let domain = parse_domain(strip_ticks(cols[1]))?;
    let format = Format::from_slug(strip_ticks(cols[2]))?;
    let foundation = cols[3].parse::<u32>().ok()?;
    let application = cols[4].parse::<u32>().ok()?;
    let reasoning = cols[5].parse::<u32>().ok()?;
    let subtotal = cols[6].parse::<u32>().ok()?;
    if foundation + application + reasoning != subtotal {
        return None;
    }
    Some(MatrixRow {
        domain,
        format,
        foundation,
        application,
        reasoning,
    })
}

fn strip_ticks(s: &str) -> &str {
    s.trim().trim_matches('`').trim()
}

fn taxonomy_version(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("**Version:**"))
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_owned)
}

pub fn domain_slug(domain: Domain) -> &'static str {
    match domain {
        Domain::Bbf => "bbf",
        Domain::Cpf => "cpf",
        Domain::Psb => "psb",
        Domain::PreClinical => "pre_clinical",
    }
}

pub(crate) fn parse_domain(s: &str) -> Option<Domain> {
    match s {
        "bbf" => Some(Domain::Bbf),
        "cpf" => Some(Domain::Cpf),
        "psb" => Some(Domain::Psb),
        "pre_clinical" => Some(Domain::PreClinical),
        _ => None,
    }
}

pub fn tier_slug(tier: Tier) -> &'static str {
    match tier {
        Tier::Foundation => "foundation",
        Tier::Application => "application",
        Tier::Reasoning => "reasoning",
    }
}
