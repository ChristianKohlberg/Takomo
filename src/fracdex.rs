//! Fractional indexing: an order key that survives concurrent inserts.
//!
//! A mindmap's siblings used to be ordered by a gapped integer `position`
//! (1000, 2000, …), which works exactly as long as one writer assigns it. Once
//! two people can insert between the same pair at the same time, they both pick
//! 1500 and the order is a coin flip that each peer may call differently.
//!
//! A fractional index is a *string* read as a fraction in base 62, ordered by
//! plain byte comparison. Between any two keys there is always another key, so
//! an insert never renumbers anything and never has to agree with anybody.
//!
//! Two invariants make it work, and both are enforced by [`is_valid`]:
//!
//! - every byte is a digit of [`DIGITS`], which is ASCII-ascending, so
//!   lexicographic byte order *is* numeric order;
//! - **no key ends in the lowest digit.** `"0"` and `""` would name the same
//!   fraction, and there is nothing to insert before `"0"`. Nothing here ever
//!   generates such a key, and [`is_valid`] refuses one arriving from a peer.
//!
//! Ordering is total only once ties are broken, and they are broken by node id
//! at the call site: two peers that independently generate the same key for
//! different nodes still agree on which comes first.
//!
//! **Nothing here trusts its input.** These keys arrive over a sync socket from
//! peers that are not trusted writers, and the arithmetic below slices strings
//! by BYTE. So [`between`] refuses anything [`is_valid`] rejects and treats it
//! as absent, rather than slicing it: an empty key, a key with a multi-byte
//! character, or a 100,000-character key would otherwise panic or overflow the
//! stack — inside the lock that guards one map's replica, taking that whole map
//! down with it.
//!
//! **This file has a twin.** `web/src/lib/fracdex.ts` implements the same
//! function, because the browser assigns keys at typing speed and the API
//! assigns them in batches. They are held together by
//! `tests/fixtures/fracdex-vectors.json`, which both test suites check. Change
//! one and the vectors fail in the other.

/// The digit alphabet, in ASCII-ascending order so byte comparison is numeric
/// comparison. 62 digits keeps keys short without reaching for characters that
/// need escaping in a URL, a JSON string, or a terminal.
pub const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// The longest key this module will accept.
///
/// A key grows by at most one digit per insert into the same gap, so reaching
/// even a fraction of this takes a run of inserts nobody performs by hand —
/// splitting one gap 64 times leaves a key under 40 characters. The cap is here
/// because [`midpoint`] recurses once per digit, and a key from a peer is only
/// as short as that peer chose to make it: without a bound, one 100,000-digit
/// key overflows the stack and aborts the process.
pub const MAX_KEY_LEN: usize = 256;

const BASE: usize = 62;

fn digit_index(c: u8) -> Option<usize> {
    DIGITS.iter().position(|&d| d == c)
}

/// Is this a key this module could have produced?
///
/// Called on every order key read out of the CRDT, because a peer is not a
/// trusted writer: the sync socket carries whatever a client sends, and a
/// malformed key would make the sibling order depend on who read it.
pub fn is_valid(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY_LEN
        && key.bytes().all(|c| digit_index(c).is_some())
        && !key.ends_with(DIGITS[0] as char)
}

/// A key strictly between `a` and `b`.
///
/// `None` means unbounded: `between(None, None)` is the first key in an empty
/// ring, `between(Some(last), None)` appends, `between(None, Some(first))`
/// prepends.
///
/// **If `a >= b` this appends after `a` instead of failing.** That ordering can
/// genuinely occur — two peers can each produce a state the other has not seen
/// yet — and a panic or an error in the middle of somebody's typing is a worse
/// answer than a node landing one place from where it was aimed. The node-id
/// tiebreak keeps the result deterministic either way.
pub fn between(a: Option<&str>, b: Option<&str>) -> String {
    // A neighbour that is not a key this module could have produced is treated
    // as no neighbour at all. It is the only safe reading — the arithmetic below
    // slices by byte and recurses per digit — and it matches what the browser
    // twin does, so both sides mint the same key from the same ring.
    let a = a.filter(|k| is_valid(k));
    let b = b.filter(|k| is_valid(k));
    match (a, b) {
        (None, None) => midpoint("", None),
        (Some(a), None) => midpoint(a, None),
        (None, Some(b)) => midpoint("", Some(b)),
        (Some(a), Some(b)) => {
            if a < b {
                midpoint(a, Some(b))
            } else {
                midpoint(a, None)
            }
        }
    }
}

/// The first key in an empty ring.
pub fn first() -> String {
    between(None, None)
}

/// `n` keys in ascending order, for seeding a ring in one pass.
///
/// Used by the migration, which converts an existing map's whole sibling ring at
/// once and would otherwise call [`between`] in a loop that lengthens the key by
/// a digit every step.
pub fn sequence(n: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(n);
    let mut prev: Option<String> = None;
    for _ in 0..n {
        let next = between(prev.as_deref(), None);
        prev = Some(next.clone());
        out.push(next);
    }
    out
}

/// The recursive core. Precondition: `a < b` when `b` is `Some`.
fn midpoint(a: &str, b: Option<&str>) -> String {
    // A shared prefix is not a choice to make — copy it and decide on the rest.
    if let Some(bs) = b {
        let common = a
            .bytes()
            .zip(bs.bytes())
            .take_while(|(x, y)| x == y)
            .count();
        if common > 0 {
            return format!(
                "{}{}",
                &a[..common],
                midpoint(&a[common..], Some(&bs[common..]))
            );
        }
    }

    // `a` empty reads as the fraction 0; `b` absent reads as 1.
    let da = a.bytes().next().and_then(digit_index).unwrap_or(0);
    let db = match b {
        Some(bs) => digit_index(bs.as_bytes()[0]).unwrap_or(BASE),
        None => BASE,
    };

    // Room between the two leading digits: take the middle one and stop.
    if db > da + 1 {
        return (DIGITS[(da + db) / 2] as char).to_string();
    }

    // The digits are adjacent, so the answer is one digit longer. Descend on
    // whichever side still has room.
    if let Some(bs) = b {
        if bs.len() > 1 {
            // `b` continues, so everything under its first digit is below it.
            return format!(
                "{}{}",
                bs.as_bytes()[0] as char,
                midpoint("", Some(&bs[1..]))
            );
        }
    }

    // Otherwise grow `a`: stay on its leading digit and look above its tail.
    let tail = if a.is_empty() { "" } else { &a[1..] };
    format!("{}{}", DIGITS[da] as char, midpoint(tail, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shared vectors. `web/src/lib/fracdex.test.ts` checks the same file,
    /// which is the only thing keeping the two implementations from drifting.
    #[test]
    fn matches_the_shared_vectors() {
        let raw = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/fracdex-vectors.json"
        ))
        .expect("fracdex vectors");
        let vectors: serde_json::Value = serde_json::from_str(&raw).expect("vectors parse");

        for case in vectors["between"].as_array().expect("between array") {
            let a = case["a"].as_str();
            let b = case["b"].as_str();
            let want = case["want"].as_str().expect("want");
            let got = between(a, b);
            assert_eq!(got, want, "between({a:?}, {b:?})");
        }

        for case in vectors["invalid"].as_array().expect("invalid array") {
            let key = case.as_str().expect("invalid key");
            assert!(!is_valid(key), "{key:?} should be refused");
        }

        for case in vectors["valid"].as_array().expect("valid array") {
            let key = case.as_str().expect("valid key");
            assert!(is_valid(key), "{key:?} should be accepted");
        }
    }

    #[test]
    fn a_new_key_always_lands_where_it_was_aimed() {
        // Every insert position in a growing ring, repeatedly — the shape that
        // breaks a gapped integer.
        let mut ring: Vec<String> = vec![first()];
        for round in 0..40 {
            let at = (round * 7) % (ring.len() + 1);
            let before = if at == 0 {
                None
            } else {
                Some(ring[at - 1].as_str())
            };
            let after = ring.get(at).map(|s| s.as_str());
            let key = between(before, after);

            assert!(is_valid(&key), "round {round}: {key:?} is not a valid key");
            if let Some(before) = before {
                assert!(
                    before < key.as_str(),
                    "round {round}: {before:?} !< {key:?}"
                );
            }
            if let Some(after) = after {
                assert!(key.as_str() < after, "round {round}: {key:?} !< {after:?}");
            }
            ring.insert(at, key);
        }

        let mut sorted = ring.clone();
        sorted.sort();
        assert_eq!(ring, sorted, "the ring must already be in key order");
    }

    #[test]
    fn repeatedly_splitting_one_gap_stays_bounded() {
        // The worst case: always insert into the same gap. The key grows by a
        // digit each time, and that is the cost — but it must grow slowly.
        let lo = first();
        let mut hi = between(Some(&lo), None);
        for _ in 0..64 {
            let mid = between(Some(&lo), Some(&hi));
            assert!(lo.as_str() < mid.as_str() && mid.as_str() < hi.as_str());
            hi = mid;
        }
        assert!(hi.len() <= 40, "key grew to {} chars: {hi}", hi.len());
    }

    #[test]
    fn sequence_is_ascending_and_valid() {
        let keys = sequence(200);
        assert_eq!(keys.len(), 200);
        for pair in keys.windows(2) {
            assert!(pair[0] < pair[1], "{:?} !< {:?}", pair[0], pair[1]);
        }
        assert!(keys.iter().all(|k| is_valid(k)));
    }

    #[test]
    fn an_out_of_order_pair_appends_rather_than_failing() {
        // Two peers can each hold a state the other has not seen. This must not
        // panic, and the result must still be a key.
        let key = between(Some("k"), Some("V"));
        assert!(is_valid(&key));
        assert!(key.as_str() > "k");
    }

    #[test]
    fn refuses_keys_it_would_never_produce() {
        assert!(!is_valid(""));
        assert!(!is_valid("0"));
        assert!(
            !is_valid("V0"),
            "a trailing lowest digit names the same fraction"
        );
        assert!(!is_valid("V!"), "punctuation is not a digit");
        assert!(!is_valid("Ü"), "non-ASCII is not a digit");
        assert!(is_valid("0V"), "a LEADING lowest digit is ordinary");
    }
}
