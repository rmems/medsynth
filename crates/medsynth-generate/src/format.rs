//! The three record formats, mirroring taxonomy §3.3.
//!
//! `medsynth-schema` exposes domains, subdomains, and tiers but not the
//! format slug; generation needs the format to pick a .jsonl shard, a schema
//! file, and a lane. Keep the strings identical to taxonomy §3.3.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A record format from taxonomy §3.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Single-best-answer multiple-choice question.
    Mcq,
    /// Clinical-style vignette with an associated question.
    CaseVignette,
    /// Worked instructional example with explicit reasoning.
    TutoringExample,
}

impl Format {
    /// All formats in taxonomy order.
    pub const fn all() -> [Format; 3] {
        [Format::Mcq, Format::CaseVignette, Format::TutoringExample]
    }

    /// The taxonomy slug used in directories, schema filenames, and JSONL
    /// shards.
    pub const fn slug(self) -> &'static str {
        match self {
            Format::Mcq => "mcq",
            Format::CaseVignette => "case_vignette",
            Format::TutoringExample => "tutoring_example",
        }
    }

    /// Parse a taxonomy slug. Case-sensitive, because the taxonomy says not
    /// to accept spelling variants.
    pub fn from_slug(slug: &str) -> Option<Self> {
        Format::all().into_iter().find(|f| f.slug() == slug)
    }

    /// Whether procedural templates may produce this format. The procedural
    /// lane covers templatable MCQs; vignettes and tutoring examples are
    /// teacher-lane formats (issue #8).
    pub const fn procedural(self) -> bool {
        matches!(self, Format::Mcq)
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.slug())
    }
}
