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
        self.with_tx(|tx| {
            tx.execute(
                "INSERT INTO tokens (id, hash, actor, scopes, projects, rate_limit, created_at, expires_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, hash, actor, scopes_raw, projects_raw, rate_limit, now, expires_at],
            )?;
            Ok(())
        })?;
        let row = TokenRow {
            id,
            actor: actor.to_string(),
            scopes: scopes.to_vec(),
            projects: projects.map(|p| p.to_vec()),
            rate_limit,
            created_at: now,
            expires_at,
            revoked_at: None,
            last_used_at: None,
        };
        Ok((row, plaintext))
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
    pub fn revoke_token(&self, id: &str) -> ApiResult<bool> {
        self.with_tx(|tx| {
            let n = tx.execute(
                "UPDATE tokens SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL",
                params![id, now_ms()],
            )?;
            Ok(n > 0)
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
