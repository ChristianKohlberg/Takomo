//! The ONE definition of "ready", as SQL both the queue and the rollups build on.
//!
//! `ready` means: a claimable state, in a project that takes work, unclaimed or
//! lease-expired, not blocked, not an expired occurrence, and not inside — or
//! above — an epic reservation. Two callers need that answer in two shapes:
//!
//! - `store::claims` filters rows by it (`WHERE`), to hand work out and to count
//!   the same set it pages over.
//! - `store::roadmap` counts rows by it (`SUM(CASE WHEN … )`), so a rollup says
//!   what the queue would actually offer.
//!
//! Those shapes are why this is emitted as SQL text rather than shared as one
//! query. What matters is that the *conditions* exist once. They did not before:
//! `roadmap` carried a copy of the blocked CTE annotated "verbatim from the ready
//! queue … so `ready` here can never drift", and then epic reservations
//! (`claim.epic_held`) were added to the queue and not to the copy. The rollup
//! went on reporting work as offerable that the queue had stopped offering —
//! exactly the failure that comment said would be worse than no rollup at all.
//! A comment cannot hold two copies together; only one definition can.
//!
//! ## Binding `now`
//!
//! Both fragments take the SQL text for the current instant, so a caller chooses
//! its own placeholder style:
//!
//! - **Positional** (`"?"`): the four references bind in the order they appear —
//!   two inside [`ready_ctes`], then two inside [`ready_conditions`]. A caller
//!   passing `"?"` must push four `now` values in that order.
//! - **Numbered** (`"?3"`): all four references resolve to one bind, and the
//!   ordering hazard disappears. Prefer this where the surrounding query is
//!   already numbered.
//!
//! ## Required aliases
//!
//! [`ready_conditions`] is written against three aliases the caller must provide:
//! `t` (tickets), `ws` (workflow_states) and `p` (projects). A `LEFT JOIN` on
//! `ws` is fine — a NULL `claimable` makes the expression false, which is the
//! right answer for a ticket whose state is not in its project's workflow.

/// The recursive CTE bodies readiness depends on, ready to follow a
/// `WITH RECURSIVE`. No leading keyword and no trailing comma, so a caller can
/// append its own CTEs after them.
///
/// `UNION`, never `UNION ALL`: it stops at an already-visited id, so a malformed
/// `parent` cycle terminates instead of hanging the query.
pub(crate) fn ready_ctes(now: &str) -> String {
    format!(
        r#"blocked(id) AS (
            SELECT DISTINCT d.ticket
            FROM deps d
            JOIN tickets b ON b.id = d.blocked_by
            JOIN workflow_states bs ON bs.project = b.project AND bs.state = b.state
            WHERE bs.terminal = 0
            UNION
            -- An epic with an active claim (leased or indefinite — NULL expiry
            -- never lapses) seeds its subtree as blocked via the parent walk
            -- below.
            SELECT e.id FROM tickets e
            WHERE e.type = 'epic' AND e.claim_holder IS NOT NULL
              AND (e.claim_expires_at IS NULL OR e.claim_expires_at > {now})
            UNION
            SELECT c.id FROM tickets c JOIN blocked ON c.parent = blocked.id
        ),
        -- Every ancestor of an actively claimed ticket. An epic in this set has
        -- live work under it, so the queue must not offer it: popping it would
        -- grant a subtree reservation over a lease someone already holds, which
        -- the claim-by-id path refuses (claim.children_held) — the queue must
        -- not hand out what the claim would refuse.
        anc_of_claimed(id) AS (
            SELECT c.parent FROM tickets c
            WHERE c.claim_holder IS NOT NULL
              AND (c.claim_expires_at IS NULL OR c.claim_expires_at > {now})
              AND c.parent IS NOT NULL
            UNION
            SELECT p.parent FROM tickets p JOIN anc_of_claimed a ON p.id = a.id
            WHERE p.parent IS NOT NULL
        )"#
    )
}

/// Readiness as one parenthesised boolean expression over `t`, `ws` and `p`.
///
/// Parenthesised so it drops into either shape without the caller reasoning
/// about operator precedence: `WHERE {expr}` for the queue, or
/// `SUM(CASE WHEN {expr} THEN 1 ELSE 0 END)` for a rollup.
///
/// Requires the CTEs from [`ready_ctes`] to be in scope.
pub(crate) fn ready_conditions(now: &str) -> String {
    format!(
        r#"(ws.claimable = 1
          AND NOT (t.type = 'epic' AND t.id IN (SELECT id FROM anc_of_claimed))
          AND t.archived_at IS NULL
          -- An archived project takes no work, so its tickets must not be
          -- offered: a queue that handed one over would be inviting an agent to
          -- claim something the archive gate then refuses to let it do anything
          -- with. Every count uses this same scope, so "n of m" stays honest.
          AND p.archived_at IS NULL
          AND (t.claim_holder IS NULL OR t.claim_expires_at <= {now})
          -- An expired scheduled occurrence is no longer live work, so it must
          -- not be handed to a worker: without this an agent calling /v1/ready
          -- would keep being given last month's review forever. It stays
          -- fetchable and claimable BY ID — only the queue stops offering it.
          AND (t.expires_at IS NULL OR t.expires_at > {now})
          AND t.id NOT IN (SELECT id FROM blocked))"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The positional contract is a counting argument, and getting it wrong binds
    // `now` to a project id. Four references, in the documented order.
    #[test]
    fn positional_form_has_exactly_four_now_references() {
        let n = ready_ctes("?").matches('?').count() + ready_conditions("?").matches('?').count();
        assert_eq!(n, 4, "callers passing \"?\" must push four `now` values");
    }

    // The numbered form collapses to ONE bind, which is why it has no ordering
    // hazard — if this ever emitted two different indices it would silently
    // reintroduce one.
    #[test]
    fn numbered_form_resolves_to_a_single_bind() {
        let sql = format!("{} {}", ready_ctes("?7"), ready_conditions("?7"));
        assert_eq!(sql.matches("?7").count(), 4);
        assert!(!sql.contains("?8"), "one index only: {sql}");
    }

    // The expression is spliced into a CASE, where an unparenthesised chain of
    // ANDs would bind to the wrong side of a surrounding OR.
    #[test]
    fn conditions_are_parenthesised_as_one_expression() {
        let c = ready_conditions("?1");
        assert!(c.starts_with('('), "must open a group: {c}");
        assert!(c.ends_with(')'), "must close it: {c}");
    }
}
