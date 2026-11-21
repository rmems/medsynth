//! Stable SHA-256 helpers.
//!
//! Record hashes cover the exact compact JSON text written as a JSONL line
//! (without the trailing newline). Registry-entry hashes cover the entry as it
//! appears inside `docs/sources.json`, so an anchor receipt pins the registry
//! state the draft was made against.
//!
//! Text documents are hashed after LF-normalization. The repository stores
//! text with `eol=lf` (`.gitattributes`), but a Windows working copy can still
//! hold CRLF until Git rewrites it; hashing raw bytes would make manifests
//! platform-dependent.

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

/// Normalize CRLF and lone CR to LF.
pub fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// SHA-256 of a UTF-8 text file after LF-normalization.
pub fn text_file_sha256(path: &Path) -> std::io::Result<String> {
    let text = std::fs::read_to_string(path)?;
    Ok(sha256_str(&normalize_newlines(&text)))
}

/// A path rendered with `/` separators, for deterministic manifests across
/// operating systems.
pub fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
