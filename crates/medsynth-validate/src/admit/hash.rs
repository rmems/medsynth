//! SHA-256 helpers, byte-compatible with `medsynth-generate`'s hashing so
//! manifest digests (config, record lines, registry entries) verify
//! identically here. Text documents hash after LF-normalization for the same
//! reason: a Windows working copy can hold CRLF until Git rewrites it.

use sha2::{Digest, Sha256};

/// Lowercase hexadecimal encoding.
fn to_hex(bytes: &[u8]) -> String {
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
