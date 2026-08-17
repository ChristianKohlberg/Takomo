//! Error-code families the contract documents as related sets.
//!
//! Call sites keep their `&str` literals — grouping lives here so an agent
//! author can discover that eight scattered codes are one vocabulary, and what
//! distinguishes the three `fence.*` codes from each other. The authoritative
//! prose for each code is `x-error-codes` in `spec/openapi.yaml`; this module
//! is the map of which routes emit which member.

/// Lease, claim, and fencing — exactly-one-claimant and zombie-worker safety.
///
/// All eight ride **409 Conflict** except where a transition check runs first
/// and a missing scope would be **403** instead (see `transition.scope` in the
/// workflow docs — a fencing complaint must not mask a scope refusal).
pub mod lease_and_fence {
    /// The ticket is already claimed by another actor until their lease expires.
    /// **When:** any mutating call on a ticket held by someone else — PATCH,
    /// comment, transition (non-human), ask (blocking), tag patch, etc.
    /// **Not** returned on claim itself (that is [`STATE`] or [`BLOCKED`]).
    pub const CLAIM_HELD: &str = "claim.held";

    /// The ticket holds no claim (and no resumable lapsed lease of yours).
    /// **When:** `POST /tickets/{id}/release` or force-release with nothing to
    /// release.
    pub const CLAIM_NONE: &str = "claim.none";

    /// The ticket's workflow state is not claimable by this caller.
    /// **When:** `POST /tickets/{id}/claim` while the state is not in the
    /// project's claimable set — unless you are resuming your own lapsed lease
    /// (`resumed: true` on the response).
    pub const CLAIM_STATE: &str = "claim.state";

    /// Open blockers keep the ticket out of the ready queue.
    /// **When:** `POST /tickets/{id}/claim` or `POST /ready/claim` on a
    /// blocked ticket.
    pub const CLAIM_BLOCKED: &str = "claim.blocked";

    /// You hold the lease but did not echo its fencing token on a mutating call.
    /// **When:** PATCH, comment, transition, ask, etc. while you are the
    /// holder — the fix is to send `fence` from the claim/heartbeat response.
    /// **Distinct from** [`STALE`]: the lease is still yours; you omitted the token.
    pub const FENCE_REQUIRED: &str = "fence.required";

    /// The fence you presented was issued once but has since been superseded, or
    /// the lease is gone (expired, released, force-released, displaced by archive).
    /// **When:** any call that checks the fence after the ticket's `fence_seq`
    /// moved on — including a zombie worker writing after its lease was taken.
    /// **Distinct from** [`INVALID`]: this value was real once; it is stale now.
    pub const FENCE_STALE: &str = "fence.stale";

    /// The fence is higher than any value this ticket has ever reached.
    /// **When:** the client fabricated, incremented, or reused another ticket's
    /// fence — a bug, not a lost lease.
    /// **Distinct from** [`STALE`]: the store never issued this number.
    pub const FENCE_INVALID: &str = "fence.invalid";

    /// You called release (or a release-shaped path) with no active lease of yours.
    /// **When:** `POST /tickets/{id}/release` without holding the ticket — pass an
    /// explicit `fence` to release anyway (admin force-release uses other codes).
    pub const RELEASE_NO_LEASE: &str = "release.no_lease";

    /// Every member of [`lease_and_fence`], for contract tests and spec pinning.
    pub const ALL: &[&str] = &[
        CLAIM_HELD,
        CLAIM_NONE,
        CLAIM_STATE,
        CLAIM_BLOCKED,
        FENCE_REQUIRED,
        FENCE_STALE,
        FENCE_INVALID,
        RELEASE_NO_LEASE,
    ];
}
