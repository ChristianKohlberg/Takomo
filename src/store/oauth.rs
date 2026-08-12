//! OAuth 2.1 authorization-server storage: dynamically registered clients,
//! single-use authorization codes, and rotating refresh tokens.
//!
//! This module holds the *state* of the flow; the HTTP shapes, the consent page
//! and the RFC 6749 error vocabulary live in `crate::api::oauth`. The split
//! matters here more than usual, because the two halves answer to different
//! specs: everything below is about not losing or double-spending a credential,
//! everything there is about saying so in the words a client parses.
//!
//! **What an OAuth access token actually is here: a normal `tk_` token with an
//! expiry.** There is no second credential type and no new branch in
//! `crate::auth` — the exchange mints a row in `tokens` whose actor, scopes,
//! project allowlist and write budget are all copied from the token the human
//! consented with (narrowed, never widened; see [`GrantedAccess`]). So an
//! OAuth-issued credential is revocable, listable and rate-limited by exactly
//! the machinery that already existed, and the hosted MCP surface it unlocks
//! needs no special case anywhere.

use super::model::{
    GrantRejection, GrantedAccess, OauthClient, OauthExchange, OauthTokens, TokenRow,
};
use super::sql::{params, OptionalExtension, Row};
use super::tokens::insert_token;
use super::Store;
use crate::error::{ApiError, ApiResult};
use crate::ids::{
    now_ms, oauth_client_id, oauth_code_plaintext, oauth_family_id, oauth_refresh_plaintext,
    pkce_s256_challenge, token_hash,
};

/// Lifetime of an issued access token. Claude refreshes reactively on a 401 and
/// proactively up to five minutes before expiry, so an hour keeps the refresh
/// traffic to roughly one exchange per connector per hour while bounding how long
/// a leaked access token is useful.
pub const ACCESS_TOKEN_TTL_SECONDS: i64 = 3600;

/// Lifetime of a refresh token. Rotated on every use, so this is the ceiling on
/// how long a *connector left idle* stays connected before the human has to
/// approve again.
pub const REFRESH_TOKEN_TTL_SECONDS: i64 = 30 * 24 * 3600;

/// Lifetime of an authorization code. Deliberately far shorter than either token
/// above: it exists only for the redirect hop from the consent screen back into
/// the client, which takes milliseconds. RFC 6749 recommends a maximum of ten
/// minutes; a minute is plenty and shrinks the window in which a code sitting in
/// a browser history or a proxy log is worth anything.
pub const AUTH_CODE_TTL_SECONDS: i64 = 60;

/// How long an expired OAuth-issued access token is kept before the sweeper
/// deletes it. Long enough that an operator investigating an incident still sees
/// yesterday's tokens in `takomo token list`, short enough that hourly refreshes
/// do not turn that list into thousands of dead rows.
pub const ISSUED_TOKEN_RETENTION_SECONDS: i64 = 24 * 3600;

/// How long a *spent* authorization code row is kept.
///
/// Not housekeeping slack: the row is what makes the replay defence reachable.
/// Delete it promptly and a replayed code matches nothing, so the exchange answers
/// `Unknown` — indistinguishable from a typo — instead of `Replayed`, and the
/// credentials that code already bought are never revoked. A code's own expiry is
/// [`AUTH_CODE_TTL_SECONDS`], which is far too short for that: it would leave the
/// defence live for one sweep tick. One access-token lifetime, the same retention
/// a rotated refresh row gets and for the same reason.
pub const SPENT_CODE_RETENTION_SECONDS: i64 = ACCESS_TOKEN_TTL_SECONDS;

/// How long a registration that has never been used is kept.
///
/// `POST /oauth/register` is unauthenticated by specification, so its rate limit
/// paces the table's growth without bounding it — this is what bounds it. "Used"
/// means an `oauth_codes` or `oauth_refresh` row still references it; a client with
/// neither, a day after registering, is a dead connection attempt or a script's
/// droppings.
///
/// So a client is protected for exactly as long as it holds a live credential. A
/// connector in use keeps a rotating refresh token and never becomes sweepable; one
/// idle past [`REFRESH_TOKEN_TTL_SECONDS`] loses that row to the same sweep and its
/// registration goes on a later tick — which is correct, because that connection is
/// already dead and a hosted client registers again when it reconnects.
pub const UNUSED_CLIENT_RETENTION_SECONDS: i64 = 24 * 3600;

/// Ceiling on registered redirect URIs per client. A hosted product needs one
/// (Claude uses `https://claude.ai/api/mcp/auth_callback`); a native one may
/// register a couple of loopback forms. Five is generous and keeps an
/// unauthenticated registration endpoint from being used to store bulk data.
pub const MAX_REDIRECT_URIS: usize = 5;

/// Compare two strings without an early exit on the first differing byte.
///
/// Used for the PKCE challenge comparison. The stakes are modest — an attacker
/// would still have to guess a 256-bit verifier — but a byte-at-a-time `==` on a
/// value derived from a secret is the kind of thing that is free to get right
/// here and awkward to retrofit later.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `"*"` (all projects) or a comma-separated allowlist — the same at-rest
/// convention as the `tokens` table, so a snapshot round-trips byte for byte.
fn projects_to_raw(projects: Option<&[String]>) -> String {
    match projects {
        None => "*".to_string(),
        Some(list) => list.join(","),
    }
}

fn projects_from_raw(raw: &str) -> Option<Vec<String>> {
    if raw == "*" {
        return None;
    }
    Some(
        raw.split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn csv_to_vec(raw: &str) -> Vec<String> {
    raw.split(',')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn row_to_client(row: &Row) -> super::sql::Result<OauthClient> {
    let uris: String = row.get("redirect_uris")?;
    Ok(OauthClient {
        client_id: row.get("client_id")?,
        client_name: row.get("client_name")?,
        // Stored as JSON so a URI containing a comma cannot split a row. Written
        // only by `register_oauth_client` below, so a parse failure means a
        // hand-edited database; an empty list then matches nothing and every
        // redirect is refused, which is the safe direction to fail.
        redirect_uris: serde_json::from_str(&uris).unwrap_or_default(),
        created_at: row.get("created_at")?,
    })
}

/// The consent snapshot as stored on a code or refresh row.
fn row_to_grant(row: &Row) -> super::sql::Result<GrantedAccess> {
    let scopes: String = row.get("scopes")?;
    let projects: String = row.get("projects")?;
    Ok(GrantedAccess {
        actor: row.get("actor")?,
        scopes: csv_to_vec(&scopes),
        projects: projects_from_raw(&projects),
        rate_limit: row.get("rate_limit")?,
        scope: row.get("scope")?,
        granted_by: row.get("granted_by")?,
    })
}

impl Store {
    /// Register a public OAuth client (RFC 7591 Dynamic Client Registration).
    pub fn register_oauth_client(
        &self,
        client_name: &str,
        redirect_uris: &[String],
    ) -> ApiResult<OauthClient> {
        let client_id = oauth_client_id();
        let now = now_ms();
        let uris_json = serde_json::to_string(redirect_uris)
            .map_err(|e| ApiError::internal(format!("cannot serialize redirect_uris: {e}")))?;
        self.with_tx(|tx| {
            tx.execute(
                "INSERT INTO oauth_clients (client_id, client_name, redirect_uris, created_at) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![client_id, client_name, uris_json, now],
            )?;
            Ok(())
        })?;
        Ok(OauthClient {
            client_id,
            client_name: client_name.to_string(),
            redirect_uris: redirect_uris.to_vec(),
            created_at: now,
        })
    }

    pub fn get_oauth_client(&self, client_id: &str) -> ApiResult<Option<OauthClient>> {
        self.with_conn(|conn| {
            let row = conn
                .query_row(
                    "SELECT client_id, client_name, redirect_uris, created_at \
                     FROM oauth_clients WHERE client_id = ?1",
                    params![client_id],
                    row_to_client,
                )
                .optional()?;
            Ok(row)
        })
    }

    /// Mint a single-use authorization code for a consented grant. Returns the
    /// plaintext, which goes back to the client in the redirect and is never
    /// stored — only its hash is.
    pub fn create_oauth_code(
        &self,
        client_id: &str,
        redirect_uri: &str,
        code_challenge: &str,
        resource: Option<&str>,
        grant: &GrantedAccess,
    ) -> ApiResult<String> {
        let plaintext = oauth_code_plaintext();
        let hash = token_hash(&plaintext);
        let now = now_ms();
        let expires_at = now + AUTH_CODE_TTL_SECONDS * 1000;
        let scopes_raw = grant.scopes.join(",");
        let projects_raw = projects_to_raw(grant.projects.as_deref());
        self.with_tx(|tx| {
            tx.execute(
                "INSERT INTO oauth_codes (code_hash, client_id, redirect_uri, code_challenge, \
                 resource, actor, scopes, projects, rate_limit, scope, granted_by, created_at, \
                 expires_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    hash,
                    client_id,
                    redirect_uri,
                    code_challenge,
                    resource,
                    grant.actor,
                    scopes_raw,
                    projects_raw,
                    grant.rate_limit,
                    grant.scope,
                    grant.granted_by,
                    now,
                    expires_at,
                ],
            )?;
            Ok(())
        })?;
        Ok(plaintext)
    }

    /// Exchange an authorization code for an access token and a refresh token.
    ///
    /// One `IMMEDIATE` transaction covers the whole step — verify, mark the code
    /// spent, mint both credentials — because a code is single-use: a crash
    /// between "spent" and "minted" would strand the client with nothing to
    /// retry. Every refusal path leaves the code exactly as it found it, except
    /// the two that are supposed to consume or invalidate it.
    pub fn exchange_oauth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        code_verifier: &str,
    ) -> ApiResult<OauthExchange> {
        let hash = token_hash(code);
        let now = now_ms();
        self.with_tx(|tx| {
            let Some((
                row_client,
                row_redirect,
                challenge,
                used_at,
                issued_family,
                expires_at,
                grant,
            )) = tx
                .query_row(
                    "SELECT client_id, redirect_uri, code_challenge, used_at, issued_family, \
                     expires_at, actor, scopes, projects, rate_limit, scope, granted_by \
                     FROM oauth_codes WHERE code_hash = ?1",
                    params![hash],
                    |row| {
                        Ok((
                            row.get::<_, String>("client_id")?,
                            row.get::<_, String>("redirect_uri")?,
                            row.get::<_, String>("code_challenge")?,
                            row.get::<_, Option<i64>>("used_at")?,
                            row.get::<_, Option<String>>("issued_family")?,
                            row.get::<_, i64>("expires_at")?,
                            row_to_grant(row)?,
                        ))
                    },
                )
                .optional()?
            else {
                return Ok(OauthExchange::Rejected(GrantRejection::Unknown));
            };

            // Replay. Someone is presenting a code that was already spent, which
            // means either a buggy client or a stolen code racing the real one —
            // and from here the two are indistinguishable. RFC 6749 §4.1.2 says
            // to revoke what the code already bought, so the winner of that race
            // does not get to keep it either: the family recorded on the code row
            // names every credential that came out of this consent.
            if used_at.is_some() {
                if let Some(family) = issued_family {
                    revoke_refresh_family(tx, &family, now)?;
                }
                return Ok(OauthExchange::Rejected(GrantRejection::Replayed));
            }
            if expires_at <= now {
                return Ok(OauthExchange::Rejected(GrantRejection::Expired));
            }
            if row_client != client_id {
                return Ok(OauthExchange::Rejected(GrantRejection::ClientMismatch));
            }
            if row_redirect != redirect_uri {
                return Ok(OauthExchange::Rejected(GrantRejection::RedirectMismatch));
            }
            // PKCE. Deliberately does NOT consume the code: the verifier is the
            // one secret a public client holds, so a mismatch means the caller is
            // not the client that started the flow — burning the code here would
            // let anyone who observes a redirect deny service to the real client.
            // Guessing past this check means guessing 256 bits.
            if !constant_time_eq(&pkce_s256_challenge(code_verifier), &challenge) {
                return Ok(OauthExchange::Rejected(GrantRejection::PkceMismatch));
            }
            // Checked last, after PKCE, so a caller who cannot prove it started the
            // flow learns nothing about the consenting token's state.
            if !consent_not_revoked(tx, &grant.granted_by)? {
                return Ok(OauthExchange::Rejected(GrantRejection::ConsentWithdrawn));
            }

            let (issued, family) = mint_grant(tx, client_id, &grant, None, now)?;
            tx.execute(
                "UPDATE oauth_codes SET used_at = ?2, issued_family = ?3 WHERE code_hash = ?1",
                params![hash, now, family],
            )?;
            Ok(OauthExchange::Issued(issued))
        })
    }

    /// Rotate a refresh token: issue a fresh access/refresh pair and retire the
    /// presented one.
    ///
    /// Rotation is not optional for a public client — the MCP authorization spec
    /// adopts OAuth 2.1's requirement — and rotation is only worth anything if
    /// reuse is *detected*. Presenting a token that was already rotated (or
    /// revoked) therefore revokes the entire family: every refresh token
    /// descended from that one consent, and every access token any of them
    /// minted. A legitimate client never does this; a thief replaying a captured
    /// token does, and this is what makes the theft self-limiting rather than
    /// permanent.
    pub fn refresh_oauth_token(&self, refresh: &str, client_id: &str) -> ApiResult<OauthExchange> {
        let hash = token_hash(refresh);
        let now = now_ms();
        self.with_tx(|tx| {
            let Some((row_client, family, rotated_at, revoked_at, expires_at, grant)) = tx
                .query_row(
                    "SELECT client_id, family, rotated_at, revoked_at, expires_at, \
                     actor, scopes, projects, rate_limit, scope, granted_by \
                     FROM oauth_refresh WHERE token_hash = ?1",
                    params![hash],
                    |row| {
                        Ok((
                            row.get::<_, String>("client_id")?,
                            row.get::<_, String>("family")?,
                            row.get::<_, Option<i64>>("rotated_at")?,
                            row.get::<_, Option<i64>>("revoked_at")?,
                            row.get::<_, i64>("expires_at")?,
                            row_to_grant(row)?,
                        ))
                    },
                )
                .optional()?
            else {
                return Ok(OauthExchange::Rejected(GrantRejection::Unknown));
            };

            // Rotated first, and the order carries meaning. A row can hold both
            // stamps — a family revoked after this row had already rotated — and
            // presenting such a token really is reuse, so `rotated_at` wins.
            //
            // Revoked but never rotated is the other story entirely: nobody
            // presented this credential twice, it was taken away. That is now a
            // routine path (`Store::revoke_token` on a derived token, or a sibling's
            // reuse detection revoking the family), and reporting it as detected
            // theft would send an operator who just ended a connector deliberately
            // into an incident response over their own action.
            if rotated_at.is_some() {
                revoke_refresh_family(tx, &family, now)?;
                return Ok(OauthExchange::Rejected(GrantRejection::Replayed));
            }
            if revoked_at.is_some() {
                // No family revoke: it is already revoked, and this is a path a
                // client will retry.
                return Ok(OauthExchange::Rejected(GrantRejection::ConnectionRevoked));
            }
            if expires_at <= now {
                return Ok(OauthExchange::Rejected(GrantRejection::Expired));
            }
            if row_client != client_id {
                return Ok(OauthExchange::Rejected(GrantRejection::ClientMismatch));
            }
            // Refused *without* retiring the presented token: this is not reuse and
            // the client did nothing wrong, so leaving its credential intact costs
            // nothing (it can mint nothing while the consent is revoked) and keeps
            // the refusal re-readable if the operator restores the token.
            if !consent_not_revoked(tx, &grant.granted_by)? {
                return Ok(OauthExchange::Rejected(GrantRejection::ConsentWithdrawn));
            }

            // Retire the presented refresh token. The access token it minted is
            // deliberately left alone to expire on its own: a client refreshes
            // *proactively*, while its current access token is still valid and
            // possibly still on requests in flight — Claude does so up to five
            // minutes before expiry — so revoking it here would fail those
            // requests to buy at most an hour of exposure. Reuse detection above
            // is where revocation earns its keep, and that path does revoke every
            // access token in the family.
            tx.execute(
                "UPDATE oauth_refresh SET rotated_at = ?2 WHERE token_hash = ?1",
                params![hash, now],
            )?;
            let (issued, _) = mint_grant(tx, client_id, &grant, Some(&family), now)?;
            Ok(OauthExchange::Issued(issued))
        })
    }

    /// Drop OAuth state that can no longer be used: expired authorization codes
    /// and spent ones past [`SPENT_CODE_RETENTION_SECONDS`], refresh tokens past
    /// expiry or long retired, the access tokens they minted once those are expired
    /// beyond [`ISSUED_TOKEN_RETENTION_SECONDS`], and registrations that were never
    /// used at all (see [`UNUSED_CLIENT_RETENTION_SECONDS`]).
    ///
    /// Only ever deletes tokens it can prove it issued itself, via the
    /// `oauth_issued` ledger — a token minted by `takomo token create` has no
    /// ledger row and is never touched. Returns how many rows went away, so the
    /// sweeper can log something meaningful.
    pub fn sweep_expired_oauth(&self) -> ApiResult<usize> {
        let now = now_ms();
        let token_cutoff = now - ISSUED_TOKEN_RETENTION_SECONDS * 1000;
        // A retired refresh row is kept for one access-token lifetime so a client
        // that rotated and immediately retried still gets `invalid_grant` (reuse
        // detected) rather than `Unknown`, which reads as "wrong server".
        let refresh_cutoff = now - ACCESS_TOKEN_TTL_SECONDS * 1000;
        let code_cutoff = now - SPENT_CODE_RETENTION_SECONDS * 1000;
        let client_cutoff = now - UNUSED_CLIENT_RETENTION_SECONDS * 1000;
        self.with_tx(|tx| {
            let mut n = 0usize;
            n += tx.execute(
                "DELETE FROM tokens WHERE id IN (SELECT token_id FROM oauth_issued) \
                 AND expires_at IS NOT NULL AND expires_at < ?1",
                params![token_cutoff],
            )?;
            n += tx.execute(
                "DELETE FROM oauth_issued WHERE token_id NOT IN (SELECT id FROM tokens)",
                [],
            )?;
            // Expiry retires an *unspent* code only. A spent one is kept on its own
            // clock, because its expiry passes within the minute and the row is the
            // replay defence — see `SPENT_CODE_RETENTION_SECONDS`.
            n += tx.execute(
                "DELETE FROM oauth_codes WHERE (used_at IS NULL AND expires_at < ?1) \
                 OR (used_at IS NOT NULL AND used_at < ?2)",
                params![now, code_cutoff],
            )?;
            n += tx.execute(
                "DELETE FROM oauth_refresh WHERE expires_at < ?1 \
                 OR (rotated_at IS NOT NULL AND rotated_at < ?2) \
                 OR (revoked_at IS NOT NULL AND revoked_at < ?2)",
                params![now, refresh_cutoff],
            )?;
            // Last, so "was this client ever used" is asked of the swept state: a
            // registration whose code and refresh rows are both gone has nothing
            // outstanding, and the two NOT IN clauses are also what keeps this from
            // deleting a row `oauth_codes.client_id` still references.
            n += tx.execute(
                "DELETE FROM oauth_clients WHERE created_at < ?1 \
                 AND client_id NOT IN (SELECT client_id FROM oauth_codes) \
                 AND client_id NOT IN (SELECT client_id FROM oauth_refresh)",
                params![client_cutoff],
            )?;
            Ok(n)
        })
    }
}

/// Mint the access/refresh pair for a grant inside an existing transaction.
///
/// `family` is `None` for a fresh consent (a new family is started) and `Some`
/// when rotating, so every token descended from one consent stays linkable —
/// which is what [`Store::refresh_oauth_token`] needs to revoke a compromised
/// chain wholesale.
fn mint_grant(
    tx: &super::sql::Tx,
    client_id: &str,
    grant: &GrantedAccess,
    family: Option<&str>,
    now: i64,
) -> ApiResult<(OauthTokens, String)> {
    let family = family.map(str::to_string).unwrap_or_else(oauth_family_id);
    let (access_row, access_plaintext): (TokenRow, String) = insert_token(
        tx,
        &grant.actor,
        &grant.scopes,
        grant.projects.as_deref(),
        grant.rate_limit,
        Some(now + ACCESS_TOKEN_TTL_SECONDS * 1000),
    )?;

    let refresh_plaintext = oauth_refresh_plaintext();
    let refresh_hash = token_hash(&refresh_plaintext);
    tx.execute(
        "INSERT INTO oauth_refresh (token_hash, family, client_id, actor, scopes, projects, \
         rate_limit, scope, granted_by, created_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            refresh_hash,
            family,
            client_id,
            grant.actor,
            grant.scopes.join(","),
            projects_to_raw(grant.projects.as_deref()),
            grant.rate_limit,
            grant.scope,
            grant.granted_by,
            now,
            now + REFRESH_TOKEN_TTL_SECONDS * 1000,
        ],
    )?;

    // The ledger. It is what lets the sweeper tell an OAuth-issued token from an
    // operator-minted one, and what lets a replayed code or a reused refresh
    // token find and revoke what it already bought.
    tx.execute(
        "INSERT INTO oauth_issued (token_id, client_id, family, refresh_hash, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![access_row.id, client_id, family, refresh_hash, now],
    )?;

    Ok((
        OauthTokens {
            access_token: access_plaintext,
            refresh_token: refresh_plaintext,
            expires_in: ACCESS_TOKEN_TTL_SECONDS,
            scope: grant.scope.clone(),
        },
        family,
    ))
}

/// Has the credential that consented been *revoked*?
///
/// The consent snapshot is deliberately a snapshot — it cannot widen when the
/// human's token gains scopes — but a snapshot that survived revocation would make
/// revocation a lie: rotation would keep minting hour-long access tokens with the
/// revoked token's full authority, renewing its own 30-day window forever, and the
/// only way to stop a connector would be to hunt down the derived token. So every
/// credential-minting path asks this first, and a `false` breaks the chain.
///
/// **Revocation only, deliberately not expiry.** The asymmetry is the point. A
/// revocation is an operator saying "this must stop", so it has to cascade. An
/// expiry is routine bookkeeping — a `--expires 90d` typed once, months ago — and
/// letting it kill a working connector would turn a forgotten flag into an outage
/// while adding nothing revocation does not already provide. A connected client is
/// meant to stay connected: the 30-day refresh window slides on every use, and this
/// check must not introduce a second clock that does not.
///
/// A missing row counts as revoked: an id with no token behind it is one that was
/// deleted, which is at least as final as one marked revoked, and there is nothing
/// left to check the delegation against.
fn consent_not_revoked(tx: &super::sql::Tx, granted_by: &str) -> ApiResult<bool> {
    let revoked_at = tx
        .query_row(
            "SELECT revoked_at FROM tokens WHERE id = ?1",
            params![granted_by],
            |row| row.get::<_, Option<i64>>("revoked_at"),
        )
        .optional()?;
    Ok(match revoked_at {
        None => false,
        Some(revoked_at) => revoked_at.is_none(),
    })
}

/// Revoke a whole refresh-token family and every access token it minted.
///
/// The family *is* the connection — every credential descended from one consent —
/// so this is the response to detected refresh-token reuse and equally the way
/// [`Store::revoke_token`] ends a single connector without touching any other.
pub(super) fn revoke_refresh_family(tx: &super::sql::Tx, family: &str, now: i64) -> ApiResult<()> {
    tx.execute(
        "UPDATE tokens SET revoked_at = ?2 WHERE revoked_at IS NULL AND id IN \
         (SELECT token_id FROM oauth_issued WHERE family = ?1)",
        params![family, now],
    )?;
    tx.execute(
        "UPDATE oauth_refresh SET revoked_at = ?2 WHERE family = ?1 AND revoked_at IS NULL",
        params![family, now],
    )?;
    Ok(())
}
