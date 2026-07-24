//! Per-question answer grants — the "answer link".
//!
//! A grant mints a bearer token (`tka_`, hashed at rest, plaintext shown once)
//! that authorizes exactly ONE write: answering the one referenced question. It
//! is what you hand an outside domain expert (a lawyer, a client) who should not
//! hold a standing token — scoped to a single question, auto-expiring, and
//! write-once (spent once the question leaves the open state, and explicitly
//! marked `used_at` after a successful answer).
//!
//! Validated on a distinct auth path (`auth::answer_auth_middleware`) that
//! reaches ONLY `/v1/answer/self*`, so a grant token can neither read arbitrary
//! data nor perform any other write.

use super::model::AnswerGrantRow;
use super::Store;
use crate::error::ApiResult;
use crate::ids::{answer_grant_id, answer_grant_token_plaintext, now_ms, token_hash};
use rusqlite::{params, OptionalExtension, Row};

/// Default answer-link lifetime when the caller omits `ttl_seconds`: 72 hours.
pub const DEFAULT_ANSWER_TTL_SECONDS: i64 = 3 * 86_400;
/// Hard cap on answer-link lifetime: 30 days.
pub const MAX_ANSWER_TTL_SECONDS: i64 = 30 * 86_400;

const GRANT_COLS: &str =
    "id, question, project, actor, expires_at, created_by, created_at, used_at, revoked_at";

fn row_to_grant(row: &Row) -> rusqlite::Result<AnswerGrantRow> {
    Ok(AnswerGrantRow {
        id: row.get("id")?,
        question: row.get("question")?,
        project: row.get("project")?,
        actor: row.get("actor")?,
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
    pub fn create_answer_grant(
        &self,
        question: &str,
        project: &str,
        actor: &str,
        expires_at: i64,
        created_by: &str,
    ) -> ApiResult<(AnswerGrantRow, String)> {
        let plaintext = answer_grant_token_plaintext();
        let hash = token_hash(&plaintext);
        let id = answer_grant_id();
        let now = now_ms();
        self.with_tx(|tx| {
            tx.execute(
                "INSERT INTO answer_grants (id, token_hash, question, project, actor, expires_at, created_by, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, hash, question, project, actor, expires_at, created_by, now],
            )?;
            Ok(())
        })?;
        Ok((
            AnswerGrantRow {
                id,
                question: question.to_string(),
                project: project.to_string(),
                actor: actor.to_string(),
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

    /// Mark a grant spent (write-once). Idempotent.
    pub fn mark_answer_grant_used(&self, id: &str) -> ApiResult<()> {
        self.with_tx(|tx| {
            tx.execute(
                "UPDATE answer_grants SET used_at = ?2 WHERE id = ?1 AND used_at IS NULL",
                params![id, now_ms()],
            )?;
            Ok(())
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
