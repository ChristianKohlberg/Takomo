# Epic claims — reserving a subtree, judged by movement instead of a lease

Claiming a ticket of type `epic` does more than lease one row: it reserves the
epic's **whole subtree** for the holder. While the claim is active:

- no other actor can claim any ticket under the epic — they get a teaching 409
  `claim.epic_held` naming the epic, the holder, and how long it has been held;
- the shared ready queue stops offering the subtree (and the epic itself), with
  `total` counted by the same scope so "n of m" stays honest;
- the **holder** keeps working children exactly as before: ordinary leased
  claims taken by id, heartbeats, fences — nothing about child claims changes.

Everything else is the claim machinery that already existed: same
`claim`/`release`/`force-release` routes and MCP tools, same fencing, same
`claimed`/`released`/`lease_revoked` events. There is no second lock concept.

## No TTL means no expiry

An epic claimed **without** `ttl_seconds` gets a claim with no expiry: the
lease returns `expires_at: null` and the claim holds until the holder releases
it (fenced, as usual) or an admin force-releases it. The sweeper never touches
it — its predicate is `claim_expires_at <= now`, and there is nothing to
compare. A heartbeat on such a claim is a no-op that returns the lease as it
stands; it never writes a TTL, so a harness beating on a schedule cannot
quietly convert "held until released" into a 15-minute lease.

An **explicit** `ttl_seconds` bounds an epic claim like any other claim
(project default/max still apply). And the ready queue only ever grants leased
claims: if `POST /v1/ready/claim` hands out an epic, the claim carries the
usual TTL — the no-expiry shape is only ever an explicit claim-by-id, never
something an agent takes without knowing.

## The reservation never displaces live work

A fresh epic claim is refused (`claim.children_held`, listing the claims)
while anyone **else** holds an active claim inside the subtree — the lock
waits for live leases, it does not fight them. The ready queue enforces the
same rule from its side: an epic with foreign claims beneath it is not
offered, because the queue must not hand out what the claim-by-id path would
refuse. The holder's own claims below don't count; the subtree is being
reserved for exactly that actor.

Deliberately **not** gated: reads, comments, body edits, and creating new
tickets under a held epic. The claim reserves *working* the subtree, not
planning it — which is exactly what makes the movement report meaningful.

## Judging a claim with no expiry: movement

`GET /v1/tickets/{id}/claim` answers the question a TTL used to answer:

```json
{
  "ticket": "tp-abc1", "type": "epic",
  "holder": "agent:w1",
  "held_since": "2026-08-16T09:12:00Z",
  "held_for_seconds": 18000,
  "expires_at": null, "indefinite": true,
  "movement": {
    "since": "2026-08-16T09:12:00Z",
    "created": 10, "closed": 5,
    "in_progress": 2, "blocked": 3, "open": 12,
    "last_activity_at": "2026-08-16T13:58:41Z",
    "idle_seconds": 132
  }
}
```

`created` and `closed` are counted **since the claim** (`closed` from the
event log, so a later reopen does not un-count it); `in_progress`, `blocked`
and `open` are current snapshots; `idle_seconds` is the time since the last
subtree event, anchored at the claim when nothing has moved at all. Five hours
held with steady movement is an agent mid-flight. Five hours held with
`idle_seconds` ≈ `held_for_seconds` is an abandoned lock — and the recovery is
the admin force-release that already exists, which bumps the fence so the
vanished holder's echoes bounce.

The server reports; it does not judge. There is no idle ceiling and no
auto-release — a supervisor, a schedule, or a human reads the movement and
decides. Entering a done/cancelled-category state auto-releases the claim like
any other, so a finished epic can never stay locked.

The same endpoint works on any ticket (on a leaf the movement counts are
simply zero), and over MCP as `takomo_claim_status`. `movement` is `null` when
the ticket is unclaimed — there is no anchor to count from — or when the claim
predates the server recording grant times (`tickets.claim_since`).

## Bits worth knowing before touching the code

- `Ticket::active_claim` returns `(holder, Option<expiry>)`; `None` expiry =
  active-indefinite. `lapsed_holder` is untouched by it: an indefinite claim
  can never lapse, so the resume-in-place path never sees one.
- The subtree gate lives in `claim_ticket` (`foreign_epic_hold_above`, an
  upward recursive CTE) and in `ready_scope` (claimed epics seed the `blocked`
  CTE; `anc_of_claimed` keeps contested epics out of the queue). Movement is
  `movement_since` in `src/store/claims.rs`.
- Every path that clears a claim also clears `claim_since` — release, force,
  expiry sweep, terminal-entry auto-release, question park/resume, project
  delete.
- Tests: the `epic_claim_*`, `ready_queue_stops_offering_*`,
  `force_release_displaces_*` and `claim_status_reports_*` block in
  `tests/api.rs`.
