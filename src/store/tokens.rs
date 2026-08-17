//! Token storage: minted/managed by the CLI, looked up by the auth middleware.

use super::model::{OauthConnection, TokenRow};
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
        user: row.get("user")?,
        created_at: row.get("created_at")?,
        expires_at: row.get("expires_at")?,
        revoked_at: row.get("revoked_at")?,
        last_used_at: row.get("last_used_at")?,
        oauth_client: None,
    })
}

/// A listed token, plus which OAuth connection it belongs to if any.
fn row_to_listed_token(row: &Row) -> rusqlite::Result<TokenRow> {
    let mut token = row_to_token(row)?;
    if let Some(client_id) = row.get::<_, Option<String>>("oauth_client_id")? {
        // `client_name` is NOT NULL DEFAULT '' on the clients table, and NULL here
        // when that registration has been swept from under a still-listed token.
        // Both mean "no name to show", so they collapse to one case.
        let client_name = row
            .get::<_, Option<String>>("oauth_client_name")?
            .filter(|name| !name.trim().is_empty());
        token.oauth_client = Some(OauthConnection {
            client_id,
            client_name,
        });
    }
    Ok(token)
}

const TOKEN_COLS: &str = "id, actor, scopes, projects, rate_limit, \"user\", created_at, \
     expires_at, revoked_at, last_used_at";

/// The same columns, aliased for the listing join.
///
/// Spelled out rather than reused with a table prefix because `oauth_clients` also
/// has a `created_at`, and `row_to_token` reads columns by name — an ambiguous name
/// would silently hand it the client's timestamp.
const TOKEN_COLS_JOINED: &str = "t.id AS id, t.actor AS actor, t.scopes AS scopes, \
     t.projects AS projects, t.rate_limit AS rate_limit, t.\"user\" AS user, \
     t.created_at AS created_at, \
     t.expires_at AS expires_at, t.revoked_at AS revoked_at, t.last_used_at AS last_used_at";

/// Insert one token row inside an existing transaction. Returns (row, plaintext).
///
/// Split out of [`Store::create_token`] so a caller that must mint a token as
/// part of a *larger* atomic step can do so without a nested transaction. The
/// OAuth code exchange (`super::oauth`) is that caller: marking the
/// authorization code spent and minting the access token it paid for have to
/// commit together, or a crash in between leaves a burnt code with no token —
/// unrecoverable for the client, since a code is single-use by design.
///
/// `user` binds the credential to a person in the directory (by id or handle),
/// and is resolved here rather than at the call sites because **it is an
/// authorization fact**: a named assignee may answer an `approve` question, and
/// this column is the only proof the caller is that person. So there is one door,
/// it takes the id or the handle a human would type, and an unknown or disabled
/// person is refused instead of stored as a dangling string.
pub(super) fn insert_token(
    tx: &rusqlite::Transaction,
    actor: &str,
    scopes: &[String],
    projects: Option<&[String]>,
    rate_limit: i64,
    expires_at: Option<i64>,
    user: Option<&str>,
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
    let user_id: Option<String> = match user {
        None => None,
        Some(raw) => {
            let person = super::users::require_user(tx, raw)?;
            if !person.active() {
                return Err(ApiError::conflict(
                    "user.disabled",
                    format!(
                        "'{}' is disabled, so a credential cannot be bound to them. Binding a token to a person is what lets them answer work addressed to them, which is exactly what disabling withdraws.",
                        person.handle
                    ),
                ));
            }
            Some(person.id)
        }
    };
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
        "INSERT INTO tokens (id, hash, actor, scopes, projects, rate_limit, created_at, expires_at, \"user\") \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![id, hash, actor, scopes_raw, projects_raw, rate_limit, now, expires_at, user_id],
    )?;
    Ok((
        TokenRow {
            id,
            actor: actor.to_string(),
            scopes: scopes.to_vec(),
            projects: projects.map(|p| p.to_vec()),
            rate_limit,
            user: user_id,
            created_at: now,
            expires_at,
            revoked_at: None,
            last_used_at: None,
            oauth_client: None,
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
        user: Option<&str>,
    ) -> ApiResult<(TokenRow, String)> {
        self.with_tx(|tx| insert_token(tx, actor, scopes, projects, rate_limit, expires_at, user))
    }

    /// Every token's metadata, each carrying the OAuth connection it belongs to.
    ///
    /// The join lives here, not in the handler or the CLI, so both listing surfaces
    /// answer the operator's real question — *which connection is this row* —
    /// identically. It is a question they could not answer before: an OAuth access
    /// token is deliberately an ordinary `tokens` row, so the ledger is the only
    /// thing that knows, and revoking the wrong row now ends the wrong connection
    /// for good.
    ///
    /// `oauth_issued.token_id` is a primary key, so the join adds at most one row
    /// per token and cannot multiply the listing.
    pub fn list_tokens(&self) -> ApiResult<Vec<TokenRow>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {TOKEN_COLS_JOINED}, i.client_id AS oauth_client_id, \
                 c.client_name AS oauth_client_name \
                 FROM tokens t \
                 LEFT JOIN oauth_issued i ON i.token_id = t.id \
                 LEFT JOIN oauth_clients c ON c.client_id = i.client_id \
                 ORDER BY t.created_at"
            ))?;
            let rows = stmt
                .query_map([], row_to_listed_token)?
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
