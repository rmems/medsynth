//! Stable SHA-256 helpers.
//!
//! Record hashes cover the exact compact JSON text written as a JSONL line
//! (without the trailing newline). Registry-entry hashes cover the exact bytes
//! of the entry as it appears inside `docs/sources.json`, so an anchor receipt
//! pins the registry state the draft was made against.

use std::path::Path;

use sha2::{Digest, Sha256};

/// Lowercase hexadecimal encoding.
pub fn to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}

/// SHA-256 of raw bytes, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    to_hex(&hasher.finalize())
}

/// SHA-256 of a UTF-8 string, lowercase hex.
pub fn sha256_str(text: &str) -> String {
    sha256_hex(text.as_bytes())
}

/// SHA-256 of a file's bytes.
pub fn file_sha256(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    Ok(sha256_hex(&bytes))
}

/// A path rendered with `/` separators, for deterministic manifests across
/// operating systems.
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
