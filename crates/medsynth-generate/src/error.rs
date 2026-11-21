//! Error type for the generation pipeline.

use std::path::PathBuf;

use crate::teacher::TeacherError;

/// A failure that stops the batch before or while writing output.
#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    /// A file could not be read or written.
    #[error("i/o error for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The run config or a target cell is invalid.
    #[error("invalid generation config: {0}")]
    Config(String),
    /// `docs/taxonomy.md` could not be parsed into the coverage matrix.
    #[error("taxonomy matrix error: {0}")]
    Matrix(String),
    /// `docs/sources.json` could not be parsed.
    #[error("anchor registry error: {0}")]
    Registry(String),
    /// The checked-in JSON Schemas could not be compiled.
    #[error("schema setup error: {0}")]
    Schema(String),
    /// The teacher lane refused or failed.
    #[error(transparent)]
    Teacher(#[from] TeacherError),
    /// JSON serialization or parsing failed.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    /// Any other run-level failure.
    #[error("{0}")]
    Other(String),
}

impl GenerateError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
