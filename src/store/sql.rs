//! Dialect shim: the seam the Postgres port moves through.
//!
//! # Why this exists
//!
//! The port is one-way — Postgres *instead of* SQLite — which sounds like it
//! removes the need for any abstraction. It does, at the END. It does not during
//! the move, and that distinction is the whole reason for this module.
//!
//! There is one `Store`. If `with_tx` hands its closure a
//! `&rusqlite::Transaction` today and a `&mut postgres::Transaction` tomorrow,
//! then all 69 closures and all 48 private fns behind them must change in ONE
//! commit or the crate does not compile. That is a big-bang port: no test signal
//! until the very end, on a store whose failure modes (wrong row ordering) are
//! silent.
//!
//! So the move goes through a scaffold, in two phases:
//!
//!   Phase 1  introduce this module, backed by rusqlite, and convert the store
//!            to it one module at a time. Every step compiles; every step is
//!            still SQLite underneath; `tests/api.rs` and `tests/mcp.rs` stay
//!            green throughout and any red is caused by the step that made it.
//!   Phase 2  swap what is behind this module for `postgres`. One file changes.
//!
//! The scaffold is temporary by intent. Once phase 2 lands it can either stay as
//! a thin naming layer or be inlined away; what it must not become is a
//! permanent two-backend abstraction, which is the expensive thing this port
//! deliberately is not.
//!
//! # What it buys beyond sequencing
//!
//! Because the shim owns the SQL string on its way to the driver, it can rewrite
//! the dialect's mechanical differences in ONE place rather than at 714 call
//! sites. [`to_pg_placeholders`] is the first of those: SQLite's `?`/`?N` become
//! Postgres's `$N` at runtime, so the port does not touch a single query for
//! that reason alone.

/// Rewrite SQLite placeholders into Postgres ones.
///
/// SQLite accepts both `?` (positional, numbered by appearance) and `?N`
/// (explicit index, and freely REUSED — `?1` may appear three times). Postgres
/// accepts only `$N`, but reuse is fine there too, so both forms map cleanly:
///
/// ```text
///   WHERE a = ?1 AND b = ?2 AND c = ?1   ->   WHERE a = $1 AND b = $2 AND c = $1
///   WHERE a = ?  AND b = ?               ->   WHERE a = $1 AND b = $2
/// ```
///
/// # The sharp edge
///
/// A `?` inside a string literal is DATA, not a placeholder, and this store has
/// them: checklist coverage globs are matched with `GLOB`, and `?` is a
/// single-character wildcard in glob syntax, so `'src/?.rs'` reaches the
/// database as a literal. Rewriting that would corrupt the query into
/// `'src/$1.rs'` and change which files a lane claims to cover — silently, and
/// in the feature whose entire purpose is that coverage claims can be checked.
///
/// So the scanner tracks single-quoted string literals (including SQL's doubled
/// `''` escape) and double-quoted identifiers, and rewrites only outside them.
/// It also skips `--` line comments and `/* */` block comments, which carry `?`
/// in this codebase's SQL more often than one would guess.
///
/// Mixing bare `?` and `?N` in one statement is rejected rather than guessed at:
/// SQLite's numbering rule for the mixed case is subtle enough that a silent
/// reinterpretation is worse than a panic during a port.
// Unused by the store until phase 2 swaps the backend — it still runs on
// SQLite, which needs no rewriting. Must be `allow`, not `expect`: the tests
// below DO call it, so under `--all-targets` an `expect(dead_code)` is
// unfulfilled and clippy errors in the opposite direction.
#[allow(dead_code)] // scaffold: wired up when the backend swaps in phase 2
pub fn to_pg_placeholders(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len() + 8);
    let mut i = 0;
    let mut next_positional = 0usize;
    let mut saw_bare = false;
    let mut saw_numbered = false;

    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            // ---- string literal: copy verbatim, honouring the '' escape ----
            '\'' => {
                out.push('\'');
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == b'\'' {
                        // A doubled quote is an escaped quote, not the end.
                        if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                            out.push_str("''");
                            i += 2;
                            continue;
                        }
                        out.push('\'');
                        i += 1;
                        break;
                    }
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            // ---- quoted identifier: copy verbatim ----
            '"' => {
                out.push('"');
                i += 1;
                while i < bytes.len() {
                    out.push(bytes[i] as char);
                    let end = bytes[i] == b'"';
                    i += 1;
                    if end {
                        break;
                    }
                }
            }
            // ---- line comment ----
            '-' if i + 1 < bytes.len() && bytes[i + 1] == b'-' => {
                while i < bytes.len() && bytes[i] != b'\n' {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            // ---- block comment ----
            '/' if i + 1 < bytes.len() && bytes[i + 1] == b'*' => {
                out.push_str("/*");
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                        out.push_str("*/");
                        i += 2;
                        break;
                    }
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            // ---- the placeholder itself ----
            '?' => {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
                if start == i {
                    saw_bare = true;
                    next_positional += 1;
                    out.push('$');
                    out.push_str(&next_positional.to_string());
                } else {
                    saw_numbered = true;
                    out.push('$');
                    out.push_str(&sql[start..i]);
                }
                assert!(
                    !(saw_bare && saw_numbered),
                    "SQL mixes bare ? and ?N placeholders, whose SQLite numbering \
                     is too subtle to translate silently. Make them all explicit. \
                     Offending SQL: {sql}"
                );
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbered_placeholders_map_directly_and_may_repeat() {
        assert_eq!(
            to_pg_placeholders("SELECT a FROM t WHERE x = ?1 AND y = ?2 AND z = ?1"),
            "SELECT a FROM t WHERE x = $1 AND y = $2 AND z = $1"
        );
    }

    #[test]
    fn bare_placeholders_are_numbered_by_appearance() {
        assert_eq!(
            to_pg_placeholders("SELECT a FROM t WHERE x = ? AND y = ? AND z = ?"),
            "SELECT a FROM t WHERE x = $1 AND y = $2 AND z = $3"
        );
    }

    /// The one that would silently change what a lane covers. `?` is a
    /// single-character wildcard in GLOB syntax, so it reaches the database as
    /// literal data inside a string.
    #[test]
    fn question_mark_inside_a_string_literal_is_data_not_a_placeholder() {
        assert_eq!(
            to_pg_placeholders("SELECT * FROM lane_globs WHERE glob GLOB 'src/?.rs' AND lane = ?1"),
            "SELECT * FROM lane_globs WHERE glob GLOB 'src/?.rs' AND lane = $1"
        );
    }

    #[test]
    fn doubled_quote_escape_does_not_end_the_literal_early() {
        // The literal is  it's ?  — the ? stays data all the way to the closing quote.
        assert_eq!(
            to_pg_placeholders("SELECT 'it''s ?' , ?1"),
            "SELECT 'it''s ?' , $1"
        );
    }

    #[test]
    fn placeholders_in_comments_are_left_alone() {
        assert_eq!(
            to_pg_placeholders("-- why ? here\nSELECT ?1"),
            "-- why ? here\nSELECT $1"
        );
        assert_eq!(
            to_pg_placeholders("/* a ? b */ SELECT ?1"),
            "/* a ? b */ SELECT $1"
        );
    }

    #[test]
    fn quoted_identifiers_are_left_alone() {
        // `shares` really does have a column named "ref".
        assert_eq!(
            to_pg_placeholders(r#"SELECT "ref" FROM shares WHERE id = ?1"#),
            r#"SELECT "ref" FROM shares WHERE id = $1"#
        );
    }

    #[test]
    fn sql_without_placeholders_is_unchanged() {
        let sql = "SELECT COUNT(*) FROM tickets";
        assert_eq!(to_pg_placeholders(sql), sql);
    }

    #[test]
    #[should_panic(expected = "mixes bare ? and ?N")]
    fn mixing_the_two_forms_is_refused_rather_than_guessed() {
        to_pg_placeholders("SELECT a FROM t WHERE x = ?1 AND y = ?");
    }

    // ---- against SQL actually in this store, not invented examples ----

    /// `projects::QUESTIONS_OF_PROJECT`, verbatim. It reuses `?1` twice and is
    /// interpolated into a larger statement that also binds `?1` — the exact
    /// shape that would break a naive left-to-right renumbering translator.
    /// Postgres allows `$1` to repeat, so the mapping is direct.
    #[test]
    fn real_sql_reusing_one_index_across_an_interpolated_fragment() {
        let frag = "project = ?1 OR ticket IN (SELECT id FROM tickets WHERE project = ?1)";
        let sql = format!(
            "SELECT COUNT(*) FROM answer_grants WHERE project = ?1 OR question IN \
             (SELECT id FROM questions WHERE {frag})"
        );
        assert_eq!(
            to_pg_placeholders(&sql),
            "SELECT COUNT(*) FROM answer_grants WHERE project = $1 OR question IN \
             (SELECT id FROM questions WHERE project = $1 OR ticket IN \
             (SELECT id FROM tickets WHERE project = $1))"
        );
    }

    /// The ready-queue predicate from `claims::ready_scope`: built by appending
    /// fragments, bare `?` throughout, with a `--` comment in the middle that
    /// itself contains no placeholder but sits between two that do.
    #[test]
    fn real_sql_from_the_dynamic_ready_queue_builder() {
        let sql = "SELECT x FROM tickets t \
                   WHERE (t.claim_holder IS NULL OR t.claim_expires_at <= ?) \
                   AND (t.expires_at IS NULL OR t.expires_at > ?) \
                   AND t.project = ? \
                   AND EXISTS (SELECT 1 FROM json_each(t.labels) WHERE json_each.value = ?) \
                   LIMIT ?";
        assert_eq!(
            to_pg_placeholders(sql),
            "SELECT x FROM tickets t \
             WHERE (t.claim_holder IS NULL OR t.claim_expires_at <= $1) \
             AND (t.expires_at IS NULL OR t.expires_at > $2) \
             AND t.project = $3 \
             AND EXISTS (SELECT 1 FROM json_each(t.labels) WHERE json_each.value = $4) \
             LIMIT $5"
        );
    }
}
