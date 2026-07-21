//! Id and token generation.

use rand::Rng;

const BASE36: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
const BASE62: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

fn random_chars(alphabet: &[u8], n: usize) -> String {
    let mut rng = rand::rng();
    (0..n)
        .map(|_| alphabet[rng.random_range(0..alphabet.len())] as char)
        .collect()
}

/// Ticket id suffix, e.g. "x7k2" → full id "rvp-x7k2".
pub fn ticket_suffix(len: usize) -> String {
    random_chars(BASE36, len)
}

/// Comment id, e.g. "c-9f3ka2xz".
pub fn comment_id() -> String {
    format!("c-{}", random_chars(BASE36, 8))
}

/// Token id (public handle for list/revoke), e.g. "tok_a8f2k1x9".
pub fn token_id() -> String {
    format!("tok_{}", random_chars(BASE36, 8))
}

/// Bearer token plaintext: `tk_` + 22 base62 chars (~131 bits).
pub fn token_plaintext() -> String {
    format!("tk_{}", random_chars(BASE62, 22))
}

/// Share id (public handle for list/revoke), e.g. "share_a8f2k1x9q7z3".
pub fn share_id() -> String {
    format!("share_{}", random_chars(BASE36, 12))
}

/// Share bearer token plaintext: `tks_` + 32 base62 chars (~190 bits). The
/// distinct `tks_` prefix keeps it visually separable from a normal `tk_`
/// token; the auth path is decided by the endpoint, not the prefix.
pub fn share_token_plaintext() -> String {
    format!("tks_{}", random_chars(BASE62, 32))
}

/// SHA-256 hex of a token plaintext (the at-rest form).
pub fn token_hash(plaintext: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(plaintext.as_bytes());
    hex(&hasher.finalize())
}

/// SHA-256 hex of arbitrary bytes (used for body-hash hints in CAS conflicts).
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Current time as unix milliseconds.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Format unix milliseconds as RFC 3339 UTC (e.g. "2026-07-19T12:00:00.123Z").
pub fn iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
