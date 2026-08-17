//! Per-question answer grants — the "answer link".
//!
//! A grant mints a bearer token (`tka_`, hashed at rest, plaintext shown once)
//! that authorizes exactly ONE write: answering the one referenced question. It
//! is what you hand an outside domain expert (a lawyer, a client) who should not
//! hold a standing token — scoped to a single question, auto-expiring, and
//! write-once. Write-once is enforced by [`spend_grant`], a conditional
//! `used_at` write made in the SAME transaction that records the answer; a
//! grant is additionally dead once its question leaves the open state
//! ([`revoke_open_grants_for_question`]), so a link minted for one answering
//! cycle can never answer a later one.
//!
//! Validated on a distinct auth path (`auth::answer_auth_middleware`) that
//! reaches ONLY `/v1/answer/self*`, so a grant token can neither read arbitrary
//! data nor perform any other write.

use super::model::AnswerGrantRow;
use super::Store;
use crate::error::ApiResult;
use crate::ids::{answer_grant_id, answer_grant_token_plaintext, now_ms, token_hash};
use rusqlite::{params, Connection, OptionalExtension, Row};

/// Revoke every still-live (unused, unrevoked) answer grant for a question.
/// Called inside the caller's transaction whenever the question leaves the open
/// state (answered / withdrawn / expired) or is reopened, so a link minted for
/// one answering cycle can never answer a later one — enforcing the write-once
/// invariant across ALL resolution paths, not just a successful self-answer.
pub(crate) fn revoke_open_grants_for_question(
    conn: &Connection,
    question: &str,
    now: i64,
) -> ApiResult<()> {
    conn.execute(
        "UPDATE answer_grants SET revoked_at = ?2 WHERE question = ?1 AND used_at IS NULL AND revoked_at IS NULL",
        params![question, now],
    )?;
    Ok(())
}

/// Spend a grant: the conditional write that IS the single-use guarantee.
///
/// Runs inside the caller's transaction — always the same transaction that
/// records the answer (`Store::answer_question_via_grant`), never one of its
/// own. Because `with_tx` serializes every mutation behind one `IMMEDIATE`
/// transaction, the `used_at IS NULL AND revoked_at IS NULL` predicate makes
/// this row the point at which concurrent attempts are ordered: exactly one
/// caller sees `true`, and a caller that sees `false` aborts the whole
/// transaction, so nothing is recorded for it. A grant is equally unspendable
/// once revoked, so a revoke landing mid-flight is honoured too.
pub(crate) fn spend_grant(conn: &Connection, id: &str, now: i64) -> ApiResult<bool> {
    let n = conn.execute(
        "UPDATE answer_grants SET used_at = ?2 WHERE id = ?1 AND used_at IS NULL AND revoked_at IS NULL",
        params![id, now],
    )?;
    Ok(n > 0)
}

/// Answer-link lifetime when neither the mint call nor the project sets one:
/// 7 days. An outside expert is asked once and answers on their own schedule,
/// so a link that dies over a weekend costs a second round of chasing; the
/// grant is single-use and scoped to one question, so the exposure a longer
/// window buys is that one question rather than a standing credential.
///
/// Precedence when minting: an explicit `ttl_seconds` on the request beats the
/// project's `answer_link_ttl_seconds`, which beats this.
pub const DEFAULT_ANSWER_TTL_SECONDS: i64 = 7 * 86_400;
/// Hard cap on answer-link lifetime: 30 days. Deliberately the same number as
/// [`crate::store::MAX_SHARE_TTL_SECONDS`] — it answers the same question ("how
/// long may a link handed to someone outside the org stay alive"), and it bounds
/// an explicit `ttl_seconds` and a project default identically, so the setting
/// can never express a lifetime a per-call `--ttl` would be refused.
pub const MAX_ANSWER_TTL_SECONDS: i64 = 30 * 86_400;

const GRANT_COLS: &str = "id, question, project, actor, \"user\", expires_at, created_by, \
     created_at, used_at, revoked_at";

fn row_to_grant(row: &Row) -> rusqlite::Result<AnswerGrantRow> {
    Ok(AnswerGrantRow {
        id: row.get("id")?,
        question: row.get("question")?,
        project: row.get("project")?,
        actor: row.get("actor")?,
        user: row.get("user")?,
        expires_at: row.get("expires_at")?,
        created_by: row.get("created_by")?,
        created_at: row.get("created_at")?,
        used_at: row.get("used_at")?,
        revoked_at: row.get("revoked_at")?,
    })
}

impl Store {
    /// Mint an answer grant for one question. Returns (row, plaintext) — the
    /// plaintext `tka_` token is shown once.
    /// `user` binds the link to a person in the directory, which is what lets it
    /// satisfy an `approve` addressed to them by *identity* rather than by a
    /// synthesized expert scope. Who may mint such a link is decided one layer up
    /// (`api::questions::mint_answer_link`), because it is an authority question
    /// rather than a storage one.
    pub fn create_answer_grant(
        &self,
        question: &str,
        project: &str,
        actor: &str,
        expires_at: i64,
        created_by: &str,
        user: Option<&str>,
    ) -> ApiResult<(AnswerGrantRow, String)> {
        let plaintext = answer_grant_token_plaintext();
        let hash = token_hash(&plaintext);
        let id = answer_grant_id();
        let now = now_ms();
        self.with_tx(|tx| {
            // Refused on an archived project, where answering is refused too: a
            // link that could only ever fail is worse than no link, because it
            // has already been mailed to someone outside the org by the time
            // anyone finds out.
            super::helpers::ensure_project_writable(tx, project)?;
            tx.execute(
                "INSERT INTO answer_grants (id, token_hash, question, project, actor, \"user\", expires_at, created_by, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![id, hash, question, project, actor, user, expires_at, created_by, now],
            )?;
            Ok(())
        })?;
        Ok((
            AnswerGrantRow {
                id,
                question: question.to_string(),
                project: project.to_string(),
                actor: actor.to_string(),
                user: user.map(str::to_string),
                expires_at,
                created_by: created_by.to_string(),
                created_at: now,
                used_at: None,
                revoked_at: None,
            },
            plaintext,
        ))
    }

    /// Look up a grant by its token's SHA-256 hash. Returns it regardless of
    /// expiry/use/revocation — the auth layer decides how to respond (so it can
    /// tell an unknown token from a spent/expired/revoked one).
    pub fn lookup_answer_grant_by_hash(&self, hash: &str) -> ApiResult<Option<AnswerGrantRow>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    &format!("SELECT {GRANT_COLS} FROM answer_grants WHERE token_hash = ?1"),
                    params![hash],
                    row_to_grant,
                )
                .optional()?;
            Ok(row)
        })
    }

    /// Load one grant by its public id (for the revoke authorization check).
    pub fn get_answer_grant(&self, id: &str) -> ApiResult<Option<AnswerGrantRow>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    &format!("SELECT {GRANT_COLS} FROM answer_grants WHERE id = ?1"),
                    params![id],
                    row_to_grant,
                )
                .optional()?;
            Ok(row)
        })
    }

    /// Revoke a grant by its id. Returns false if no such not-yet-revoked grant.
    pub fn revoke_answer_grant(&self, id: &str) -> ApiResult<bool> {
        self.with_tx(|tx| {
            let n = tx.execute(
                "UPDATE answer_grants SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![id, now_ms()],
            )?;
            Ok(n > 0)
        })
    }
}
