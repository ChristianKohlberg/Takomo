# Scheduled tickets

Some work is periodic: a weekly review, a monthly key rotation, a nightly backup check. Without a schedule somebody has to remember it, or an external cron has to POST `/v1/tickets` from its own host with its own long-lived token and its own retry logic.

A **schedule** is a rule that creates an ordinary ticket on a cadence. The ticket links back to the schedule and to nothing else. That is the whole feature — plus one new idea, expiry, which is what makes it more than a cron with a nicer UI.

- The cadence grammar is [`spec/schedule-format.md`](../spec/schedule-format.md).
- The HTTP contract is [`spec/openapi.yaml`](../spec/openapi.yaml) under the `schedules` tag.

## What a schedule produces

An **ordinary ticket**. Claimable, leasable, fenced, moved through the project's own workflow, visible on `/board`, offered by `/v1/ready`. It carries three extra columns:

| Field | Meaning |
|---|---|
| `schedule` | the rule that made it — provenance and a link back |
| `occurrence` | the calendar slot it stands for |
| `expires_at` | when it stops counting as **live** work |

Nothing about the board, the event log or the SSE stream had to learn a new shape: firing emits the usual `ticket_created` alongside `schedule_fired`.

### Occurrences do not know about each other

There is no overlap policy, no predecessor lookup, no dependency between the tickets one schedule makes. `expires_at` is computed from the *cadence* at creation, never by reading a sibling — which is what makes that independence true rather than merely intended.

It also means exactly one ticket per slot is **structural**, not defensive: `UNIQUE(tickets.schedule, tickets.occurrence)`. Two sweeps, a manual run racing the timer, and a restart mid-tick all converge on one ticket, because the second insert cannot land. A duplicate attempt reports `created: false` — the guarantee doing its job, not an error.

## Expiry: "not fulfilled" instead of a stale `todo`

Takomo has no notion of a ticket expiring on its own — a lease expires, a question expires, but a ticket sits in `todo` forever. So without expiry, "nobody did last week's review" and "this week's is still fresh" would be the same row.

An occurrence's deadline is **the next occurrence**. Three outcomes, all derived at read time:

| Outcome | Derived when |
|---|---|
| `done` | the state is terminal |
| `open` | not terminal, deadline not passed — this is the one to do |
| `not_fulfilled` | not terminal, deadline passed — nobody finished it in time |

**Expiry changes no state.** An expired occurrence is not archived, cancelled or transitioned, because any of those would need a legal edge in every project's workflow and a scheduler must never be able to hit an illegal-transition wall. What it does:

- drops out of `GET /v1/ready`, so no agent is handed last month's review;
- stays fetchable and claimable **by id**;
- renders as `not fulfilled` on `/board` and on `/schedules`.

### Cleaning up is a maintenance agent's job

Nothing on the server closes an expired occurrence. Find them with:

```sh
curl -s -H "Authorization: Bearer $TOKEN" \
  "$URL/v1/tickets?expired=true&project=demo"
```

…and close them out through the ordinary API. Which means the maintenance job can be **a schedule itself**:

```json
{
  "project": "demo",
  "name": "Tidy expired scheduled tickets",
  "cadence": { "every": "week", "at": "07:00" },
  "template": { "title": "Tidy expired scheduled tickets — {week}" }
}
```

## Agents propose, humans activate

Creating a schedule needs only `write`, because an agent noticing "this keeps coming back every week" and proposing a cadence is a behaviour worth having. `takomo_schedule_new` over MCP is exactly that.

What an agent creates is **inert**. Unless the project turns the flag off, it lands `pending` with `next_slot = NULL` — and the sweep's index is partial (`WHERE next_slot IS NOT NULL`), so the sweeper cannot see the row at all. Inert by construction rather than by a check somewhere.

**Activating needs `human`.** That is the escalation this closes: a schedule outlives the token that made it, so a `write` credential able to start one could keep writing tickets long after it was revoked. A `human` caller's own schedule is born active — asking someone to approve their own proposal is theatre when they already hold the authority the flag protects.

```sh
# the operator's switch, admin-scoped like every project setting
curl -sX PUT -H "Authorization: Bearer $ADMIN" -H 'Content-Type: application/json' \
  -d '{"required": false}' "$URL/v1/projects/demo/schedule-approval"
```

Default is on. Turn it off to let a fleet schedule its own recurring work.

## The page

`/schedules` is rows, not columns — and that is the design decision worth stating. The board sorts by state into columns; a schedule's content is a **history**, so forcing cadences into columns would throw away the axis carrying all the meaning.

Three groups in fixed order: waiting for you, active, stopped. Each row shows the cadence, the last eight occurrences as a strip with their outcomes, the tally, and the next slot. A proposal additionally shows the ticket it *would* create — approving a cadence without seeing its ticket is approving a name.

The strip is what earns the page. `done done not_fulfilled not_fulfilled done open` says the cadence has stopped working, and no single ticket could have told you.

On `/board`, a scheduled card carries a quiet `↻` chip with the schedule id, and an expired one carries a `not fulfilled` marker — the only place a reader learns it has stopped counting as live work, since nothing transitioned it.

## From the shell

The server is not the root of trust; shell access is ([`spec/auth.md`](../spec/auth.md)). So an operator can steer schedules without minting a `human` token:

```sh
takomo schedule list --project demo
takomo schedule activate sch-9f3ka2xz     # a pending proposal
takomo schedule pause sch-9f3ka2xz
takomo schedule resume sch-9f3ka2xz       # from the NEXT slot; never backfills
takomo schedule run sch-9f3ka2xz          # off-cycle, without shifting the cadence
```

## Two limits worth knowing before you hit them

**A missed slot leaves a gap, not a row.** Only the most recent due slot fires — materializing the ones that passed while nothing was running would create tickets that are already expired, which is work with no output. The history therefore has a hole where the downtime was, and a `schedule_missed` event records how many slots were passed over, so a gap reads as "nothing was running" rather than "nothing was scheduled".

**Outcomes are derived, so history is not frozen.** Re-opening a July ticket in October changes what July looks like on the strip. That was the trade for deleting a whole occurrences table: an outcome that is computed can never disagree with its ticket.

## Timing

A schedule fires within one sweeper tick of its slot — 10 s by default (`TAKOMO_SWEEP_SECONDS`). Second precision is not promised, and the finest cadence is daily: a ticket a fleet churns through every few minutes is a worker loop, not tracked work.
