//! Recorded fulfillment for the pipeline's external checks.
//!
//! The admission binary never calls a model or a source resolver itself —
//! the same rule the teacher lane follows. Each external check writes its
//! request to the fulfillment directory as `<key>.<kind>.request.json` and
//! the harness drops the verdict at `<key>.<kind>.response.json`. `kind` is
//! `solve` (stage 2), `verify` (stages 3–4), or `review` (stage 5); `key`
//! is the SHA-256 of the exact request-file bytes, matching the teacher
//! lane's convention so the same harness pattern fulfills all of them.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use super::hash::sha256_str;

/// A per-write unique staging-file tag: process id plus a monotonically
/// increasing counter, so two concurrent writers staging the same target
/// never share — or clean up — each other's temp file.
pub fn tmp_tag() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

/// Which external check a fulfillment file belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// Stage-2 blind key re-derivation.
    Solve,
    /// Stage-3 claim support / stage-4 distractor screen.
    Verify,
    /// Stage-5 accuracy review.
    Review,
}

impl RequestKind {
    /// The filename segment: `<key>.<suffix>.request.json`.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Solve => "solve",
            Self::Verify => "verify",
            Self::Review => "review",
        }
    }
}

/// The request body a fulfiller hashes to find its task.
pub fn request_text(body: &Value) -> String {
    serde_json::to_string_pretty(body).expect("request bodies serialize")
}

/// The fulfillment key: SHA-256 of the exact request-file bytes.
pub fn request_key(body: &Value) -> String {
    sha256_str(&request_text(body))
}

/// Result of looking for a response after the request is written.
#[derive(Debug)]
pub enum Exchange {
    /// The response file exists and parsed.
    Response { value: Value },
    /// No response yet — a transient, infrastructure-level miss. The record
    /// is held, never rejected (contract §8: outages are not defects).
    Unavailable { key: String },
    /// The response file exists but is not usable JSON — also transient.
    Malformed { key: String, detail: String },
}

/// A fulfillment directory shared with the external harness.
pub struct Fulfillment {
    dir: PathBuf,
}

impl Fulfillment {
    pub fn new(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    /// Write the request file (idempotent — the same body always lands at
    /// the same key) and look for its response. I/O failures other than a
    /// missing response are operational errors.
    pub fn exchange(&self, kind: RequestKind, body: &Value) -> std::io::Result<Exchange> {
        let key = request_key(body);
        let request_path = self
            .dir
            .join(format!("{key}.{}.request.json", kind.suffix()));
        let response_path = self
            .dir
            .join(format!("{key}.{}.response.json", kind.suffix()));
        // Stage then rename: a pre-existing symlink at the request path is
        // replaced by the rename, never followed to truncate its target.
        // create_new for the staging file itself so a planted link fails
        // loudly instead of redirecting the write.
        let tmp_path = request_path.with_extension(format!("tmp-{}", tmp_tag()));
        let write = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp_path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, request_text(body).as_bytes()))
            .and_then(|()| std::fs::rename(&tmp_path, &request_path));
        if let Err(e) = write {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
        match std::fs::read_to_string(&response_path) {
            Ok(text) => match serde_json::from_str(&text) {
                Ok(value) => Ok(Exchange::Response { value }),
                Err(e) => Ok(Exchange::Malformed {
                    key,
                    detail: format!("{e}"),
                }),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Exchange::Unavailable { key }),
            // Bytes that are not UTF-8 are a malformed response, not an
            // outage — the record holds, it does not sink the run.
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Ok(Exchange::Malformed {
                key,
                detail: format!("response is not UTF-8: {e}"),
            }),
            Err(e) => Err(e),
        }
    }
}
