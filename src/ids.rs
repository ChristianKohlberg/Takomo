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

/// Question id, e.g. "q-9f3ka2xz".
pub fn question_id() -> String {
    format!("q-{}", random_chars(BASE36, 8))
}

/// Question-thread message id, e.g. "qm-9f3ka2xz".
pub fn question_message_id() -> String {
    format!("qm-{}", random_chars(BASE36, 8))
}

/// Promotion id, e.g. "pr-9f3ka2xz".
pub fn promotion_id() -> String {
    format!("pr-{}", random_chars(BASE36, 8))
}

/// Initiative id, e.g. "ini-9f3ka2xz".
pub fn initiative_id() -> String {
    format!("ini-{}", random_chars(BASE36, 8))
}

/// Initiative entry id, e.g. "ie-9f3ka2xz".
pub fn initiative_entry_id() -> String {
    format!("ie-{}", random_chars(BASE36, 8))
}

/// Mindmap id, e.g. "mm-9f3ka2xz".
pub fn mindmap_id() -> String {
    format!("mm-{}", random_chars(BASE36, 8))
}

/// Mindmap node id, e.g. "mn-9f3ka2xz".
pub fn mindmap_node_id() -> String {
    format!("mn-{}", random_chars(BASE36, 8))
}

/// Schedule id, e.g. "sch-9f3ka2xz".
pub fn schedule_id() -> String {
    format!("sch-{}", random_chars(BASE36, 8))
}

/// Environment id, e.g. "env-9f3ka2xz". Agents address an environment by its
/// `slug` ("staging"); this is the stable key everything else references.
pub fn environment_id() -> String {
    format!("env-{}", random_chars(BASE36, 8))
}

/// Tag (project entity — person, component, …) id, e.g. "tag-9f3ka2xz".
pub fn tag_id() -> String {
    format!("tag-{}", random_chars(BASE36, 8))
}

/// User (a person in the directory) id, e.g. "usr-9f3ka2xz".
///
/// Distinct from [`tag_id`] on purpose. A `person:ada` tag is reference metadata
/// scoped to one project; a user is the person themselves, global to the server,
/// and work can be addressed to them. See src/store/users.rs.
pub fn user_id() -> String {
    format!("usr-{}", random_chars(BASE36, 8))
}

/// Workflow library entry id, e.g. "wf-9f3ka2xz".
pub fn workflow_entry_id() -> String {
    format!("wf-{}", random_chars(BASE36, 8))
}

/// Release id, e.g. "rel-9f3ka2xz".
pub fn release_id() -> String {
    format!("rel-{}", random_chars(BASE36, 8))
}

/// Checklist check id, e.g. "check-9f3ka2xz".
pub fn check_id() -> String {
    format!("check-{}", random_chars(BASE36, 8))
}

/// Checklist case id, e.g. "case-9f3ka2xz".
pub fn case_id() -> String {
    format!("case-{}", random_chars(BASE36, 8))
}

/// Recorded-verdict id, e.g. "cv-9f3ka2xz".
pub fn verdict_id() -> String {
    format!("cv-{}", random_chars(BASE36, 8))
}

/// Checklist policy id, e.g. "clp-9f3ka2xz".
pub fn checklist_policy_id() -> String {
    format!("clp-{}", random_chars(BASE36, 8))
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

/// Answer-grant id (public handle for revoke), e.g. "ag_a8f2k1x9q7z3".
pub fn answer_grant_id() -> String {
    format!("ag_{}", random_chars(BASE36, 12))
}

/// Answer-grant bearer token: `tka_` + 32 base62 chars (~190 bits). The distinct
/// `tka_` prefix keeps it visually separable from a normal `tk_` token and a
/// read-only share `tks_` token; the auth path is decided by the endpoint.
pub fn answer_grant_token_plaintext() -> String {
    format!("tka_{}", random_chars(BASE62, 32))
}

/// Share bearer token plaintext: `tks_` + 32 base62 chars (~190 bits). The
/// distinct `tks_` prefix keeps it visually separable from a normal `tk_`
/// token; the auth path is decided by the endpoint, not the prefix.
pub fn share_token_plaintext() -> String {
    format!("tks_{}", random_chars(BASE62, 32))
}

/// OAuth client id, e.g. "oc_a8f2k1x9q7z3n4m6". Public, not a secret: takomo
/// registers OAuth clients as *public* clients (no `client_secret`), because a
/// hosted MCP client cannot keep one and PKCE is what actually binds the code to
/// the requester.
pub fn oauth_client_id() -> String {
    format!("oc_{}", random_chars(BASE36, 16))
}

/// OAuth authorization code: `tkc_` + 32 base62 chars (~190 bits). Single-use and
/// short-lived; hashed at rest like every other credential here.
pub fn oauth_code_plaintext() -> String {
    format!("tkc_{}", random_chars(BASE62, 32))
}

/// OAuth refresh token: `tkr_` + 32 base62 chars (~190 bits).
pub fn oauth_refresh_plaintext() -> String {
    format!("tkr_{}", random_chars(BASE62, 32))
}

/// Refresh-token *family* id, e.g. "of_a8f2k1x9q7z3". Every refresh token minted
/// from one consent shares it, which is what makes rotation-reuse detection
/// possible: presenting an already-rotated token revokes the whole family rather
/// than just the one row (OAuth 2.1 for public clients).
pub fn oauth_family_id() -> String {
    format!("of_{}", random_chars(BASE36, 12))
}

/// base64url without padding (RFC 4648 §5) — the encoding PKCE code challenges
/// use. Hand-rolled rather than pulling in a base64 crate: it is a dozen lines
/// for the one alphabet needed here, and [`pkce_s256_challenge`] below is pinned
/// against RFC 7636's own test vector, so a mistake in it cannot go unnoticed.
pub fn base64url_nopad(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        // Pack the chunk into a 24-bit big-endian accumulator; a short final
        // chunk leaves the missing bytes as zero bits, and emitting only
        // `chunk.len() + 1` characters is precisely what "no padding" means.
        let mut acc = 0u32;
        for (i, b) in chunk.iter().enumerate() {
            acc |= (*b as u32) << (16 - 8 * i);
        }
        for i in 0..=chunk.len() {
            let idx = ((acc >> (18 - 6 * i)) & 0x3f) as usize;
            out.push(ALPHABET[idx] as char);
        }
    }
    out
}

/// The PKCE `S256` code challenge for a verifier: base64url(SHA-256(verifier)),
/// unpadded (RFC 7636 §4.2). This is the check that binds an authorization code
/// to whoever started the flow, and it is the *only* client authentication a
/// public client has — so it is compared, never trusted.
pub fn pkce_s256_challenge(verifier: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    base64url_nopad(&hasher.finalize())
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

/// Unit tests, unusually for this crate (the weight lives in `tests/`, because
/// almost everything here is reachable only through HTTP). These two functions
/// are the exception: pure, HTTP-free, and load-bearing for PKCE, where a subtly
/// wrong encoder would not fail loudly — it would make every authorization code
/// exchange fail with `invalid_grant` and no hint as to why. Pinning them against
/// the RFCs' own vectors is cheaper than debugging that from a client's error log.
#[cfg(test)]
mod encoding_tests {
    use super::{base64url_nopad, pkce_s256_challenge};

    /// RFC 4648 §10 test vectors, minus the padding. The three cases per group of
    /// three input bytes are what an off-by-one in the tail loop gets wrong.
    #[test]
    fn base64url_matches_rfc4648_vectors() {
        assert_eq!(base64url_nopad(b""), "");
        assert_eq!(base64url_nopad(b"f"), "Zg");
        assert_eq!(base64url_nopad(b"fo"), "Zm8");
        assert_eq!(base64url_nopad(b"foo"), "Zm9v");
        assert_eq!(base64url_nopad(b"foob"), "Zm9vYg");
        assert_eq!(base64url_nopad(b"fooba"), "Zm9vYmE");
        assert_eq!(base64url_nopad(b"foobar"), "Zm9vYmFy");
    }

    /// The URL-safe alphabet is the whole point of base64**url**: byte 0xfb
    /// encodes to `-` and 0xff to `_`, where standard base64 would emit `+`
    /// and `/` and break inside a query string.
    #[test]
    fn base64url_uses_the_url_safe_alphabet() {
        assert_eq!(base64url_nopad(&[0xfb, 0xff]), "-_8");
        assert!(!base64url_nopad(&[0xff; 32]).contains(['+', '/', '=']));
    }

    /// RFC 7636 appendix B's worked example, verifier to challenge.
    #[test]
    fn pkce_challenge_matches_rfc7636_vector() {
        assert_eq!(
            pkce_s256_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
