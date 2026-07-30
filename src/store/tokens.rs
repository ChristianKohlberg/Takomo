//! Token storage: minted/managed by the CLI, looked up by the auth middleware.

use super::model::TokenRow;
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{now_ms, token_hash, token_id, token_plaintext};
use rusqlite::{params, OptionalExtension, Row};

fn row_to_token(row: &Row) -> rusqlite::Result<TokenRow> {
    let scopes_raw: String = row.get("scopes")?;
    let projects_raw: String = row.get("projects")?;
    Ok(TokenRow {
        id: row.get("id")?,
        actor: row.get("actor")?,
        scopes: scopes_raw
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
        projects: if projects_raw == "*" {
            None
        } else {
            Some(
                projects_raw
                    .split(',')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
            )
        },
        rate_limit: row.get("rate_limit")?,
        created_at: row.get("created_at")?,
        expires_at: row.get("expires_at")?,
        revoked_at: row.get("revoked_at")?,
        last_used_at: row.get("last_used_at")?,
    })
}

const TOKEN_COLS: &str =
    "id, actor, scopes, projects, rate_limit, created_at, expires_at, revoked_at, last_used_at";

/// Insert one token row inside an existing transaction. Returns (row, plaintext).
///
/// Split out of [`Store::create_token`] so a caller that must mint a token as
/// part of a *larger* atomic step can do so without a nested transaction. The
/// OAuth code exchange (`super::oauth`) is that caller: marking the
/// authorization code spent and minting the access token it paid for have to
/// commit together, or a crash in between leaves a burnt code with no token —
/// unrecoverable for the client, since a code is single-use by design.
pub(super) fn insert_token(
    tx: &rusqlite::Transaction,
    actor: &str,
    scopes: &[String],
    projects: Option<&[String]>,
    rate_limit: i64,
    expires_at: Option<i64>,
) -> ApiResult<(TokenRow, String)> {
    if actor.trim().is_empty() {
        return Err(ApiError::validation(
            "token.actor",
            "actor must be non-empty",
        ));
    }
    if scopes.is_empty() {
        return Err(ApiError::validation(
            "token.scopes",
            "at least one scope is required",
        ));
    }
    let plaintext = token_plaintext();
    let hash = token_hash(&plaintext);
    let id = token_id();
    let now = now_ms();
    let scopes_raw = scopes.join(",");
    let projects_raw = match projects {
        None => "*".to_string(),
        Some(list) => list.join(","),
    };
    tx.execute(
        "INSERT INTO tokens (id, hash, actor, scopes, projects, rate_limit, created_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![id, hash, actor, scopes_raw, projects_raw, rate_limit, now, expires_at],
    )?;
    Ok((
        TokenRow {
            id,
            actor: actor.to_string(),
            scopes: scopes.to_vec(),
            projects: projects.map(|p| p.to_vec()),
            rate_limit,
            created_at: now,
            expires_at,
            revoked_at: None,
            last_used_at: None,
        },
        plaintext,
    ))
}

impl Store {
    /// Mint a token. Returns (row, plaintext) — the plaintext is shown once.
    pub fn create_token(
        &self,
        actor: &str,
        scopes: &[String],
        projects: Option<&[String]>,
        rate_limit: i64,
        expires_at: Option<i64>,
    ) -> ApiResult<(TokenRow, String)> {
        self.with_tx(|tx| insert_token(tx, actor, scopes, projects, rate_limit, expires_at))
    }

    pub fn list_tokens(&self) -> ApiResult<Vec<TokenRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TOKEN_COLS} FROM tokens ORDER BY created_at"
            ))?;
            let rows = stmt
                .query_map([], row_to_token)?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
    }

    /// Revoke by token id. Returns false if no such token.
    ///
    /// **If the token was issued by the OAuth flow, this ends that connection** —
    /// not just the one credential. Marking the row alone would not: the client
    /// holds a refresh token, so it answers the resulting 401 by rotating and is
    /// back inside a round trip, with a fresh 30-day window. So the refresh family
    /// goes too, and no rotation can mint a replacement.
    ///
    /// The `oauth_issued` ledger is the join, because an OAuth access token is
    /// deliberately an ordinary `tokens` row — that is what keeps `crate::auth`
    /// free of a second credential type — which leaves nothing on the row itself to
    /// recognize. The ledger exists precisely to answer "which connection did this
    /// token belong to".
    ///
    /// A token with no ledger row is by definition not OAuth-issued and nothing
    /// changes for it. Worth stating because this is the one path behind both
    /// `takomo token revoke` and `DELETE /v1/tokens/{id}`: an operator revoking a
    /// hand-minted worker token must not have anything else happen.
    pub fn revoke_token(&self, id: &str) -> ApiResult<bool> {
        let now = now_ms();
        self.with_tx(|tx| {
            let n = tx.execute(
                "UPDATE tokens SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![id, now],
            )?;
            if n == 0 {
                return Ok(false);
            }
            let family: Option<String> = tx
                .query_row(
                    "SELECT family FROM oauth_issued WHERE token_id = ?1",
                    params![id],
                    |row| row.get("family"),
                )
                .optional()?;
            if let Some(family) = family {
                super::oauth::revoke_refresh_family(tx, &family, now)?;
            }
            Ok(true)
        })
    }

    /// Look up an active (non-revoked, non-expired) token by plaintext hash.
    pub fn lookup_token(&self, hash: &str) -> ApiResult<Option<TokenRow>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    &format!("SELECT {TOKEN_COLS} FROM tokens WHERE hash = ?1"),
                    params![hash],
                    row_to_token,
                )
                .optional()?;
            Ok(row)
        })
    }

    /// Update last_used_at (called at most ~once a minute per token).
    pub fn touch_token(&self, id: &str) -> ApiResult<()> {
        self.with_tx(|tx| {
            tx.execute(
                "UPDATE tokens SET last_used_at = ?2 WHERE id = ?1",
                params![id, now_ms()],
            )?;
            Ok(())
        })
    }
}
