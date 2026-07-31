# Schedule cadence format

A schedule creates an ordinary ticket on a cadence. The ticket links back to the schedule and to nothing else — no dependency, no ordering, no predecessor. The cadence is data, not code: a JSON object validated on write and turned into occurrence instants by [`src/schedule.rs`](../src/schedule.rs), which is pure and carries the unit tests for every calendar edge below.

This document covers the **cadence** only. The schedule row that wraps it (its template, timezone, status, `starts_at`) is in the REST contract, [`spec/openapi.yaml`](openapi.yaml).

## Format

```json
{
  "every": "week",
  "interval": 1,
  "on": ["mon"],
  "at": "09:00",
  "tz": "Europe/Berlin"
}
```

| Field | Required | Meaning |
|---|---|---|
| `every` | yes | `day`, `week` or `month`. |
| `interval` | no, default `1` | Repeat every N units. `1`–`52`. |
| `on` | `week` only | Weekday tokens: `mon` `tue` `wed` `thu` `fri` `sat` `sun`. At least one. |
| `day` | `month` only, default `1` | Day of month, `1`–`31`. Clamped to the month's length. |
| `at` | yes | Local wall-clock time, `HH:MM`, 24-hour, zero-padded. |
| `tz` | no, default `UTC` | IANA zone name, e.g. `Europe/Berlin`. |

Unknown fields are **refused**, and so is a field that does not apply to the unit. `{"every": "day", "on": ["mon"]}` is a 422 rather than an ignored extra: accepting it would fire seven times a week when the author asked for one. This is the same reasoning as `deny_unknown_fields` in [`spec/workflow-format.md`](workflow-format.md), where a mistyped `require:` would silently delete an approval gate.

```json
{ "every": "day",   "at": "06:30" }
{ "every": "month", "day": 1, "at": "09:00", "tz": "Europe/Berlin" }
{ "every": "week",  "interval": 2, "on": ["mon"], "at": "09:00" }
{ "every": "week",  "on": ["mon", "thu"], "at": "09:00" }
```

The finest cadence is daily. There is deliberately no sub-daily unit: a ticket a fleet churns through every few minutes is a worker loop, not a task store's job.

## Why not cron

Cron is what most callers will reach for, and a cadence is not one. Rather than maintain a second dialect, a cron-shaped string is answered with its translation:

```
POST /v1/schedules   {"cadence": "0 9 * * mon", …}   →  422

  takomo cadences are declarative objects, not cron strings. The equivalent of
  '0 9 * * mon' is {"every":"week","on":["mon"],"at":"09:00","tz":"UTC"}. Cron
  carries no timezone, so set tz explicitly (IANA name, e.g. "Europe/Berlin")
  or it defaults to UTC.
```

Numeric and named day-of-week fields both translate (`0` and `7` are Sunday, as in every cron). An expression that varies the minute or hour — `*/15 * * * *` — has no cadence equivalent and says so instead of guessing.

## Slots, and the two clock anomalies

A slot is computed **in local time**, then converted to an instant. That is the whole reason for the `chrono-tz` dependency: "every Monday at 09:00" has to stay 09:00 across a daylight-saving boundary, and a UTC-only implementation would drift it to 10:00 for half of every year.

Local time is not a total function, so two cases need a stated rule:

| Case | Behaviour | Why |
|---|---|---|
| **Spring forward** — the requested wall clock does not exist (`02:30` on a day the clocks jump `02:00 → 03:00`) | Clamp forward to the first minute that does exist, here `03:00` | Never silently drop an occurrence. A missed occurrence must be a decision somebody made, not an artifact of the calendar. |
| **Fall back** — the requested wall clock happens twice | Fire on the earlier one | Deterministic rather than whichever the library returned. |
| `day: 31` in a short month | Clamp to the last day — 28, 29 in a leap year, or 30 | "The 31st" plainly means month-end to whoever wrote it. |

## Interval counts from the anchor

`interval` is counted from the schedule's anchor (its `starts_at`, or its creation time), not from calendar parity. So `{"every": "week", "interval": 2, "on": ["mon"]}` lands on the same Mondays a year later, instead of flipping whenever a year has 53 weeks.

- `day` — whole days since the anchor's date, modulo `interval`.
- `week` — whole weeks between the Mondays of the anchor's and the candidate's weeks, modulo `interval`.
- `month` — calendar months since the anchor's month, modulo `interval`.

## Expiry: there is no field for it

An occurrence's deadline is **the next occurrence**. When a ticket is created for a slot it is stamped with `expires_at = next_slot_after(slot)`, computed from the cadence alone — nothing reads a sibling ticket. That is what keeps occurrences independent of one another, and it is why the cadence has no `expires_after`.

The resulting three outcomes are derived at read time, never stored:

| Outcome (`ScheduleOccurrence.outcome`) | Derived when |
|---|---|
| `done` | the ticket's state is terminal |
| `open` | not terminal, and `now <= expires_at` |
| `not_fulfilled` | not terminal, and `now > expires_at` |

Expiry changes no state and needs no workflow edge: an expired ticket is not archived, cancelled or transitioned. It drops out of `GET /v1/ready` so no agent is handed last month's review, and is rendered as not fulfilled. Closing them out is ordinary work for a maintenance agent — which can itself be a schedule. `GET /v1/tickets?expired=true` is how that agent finds them; `GET /v1/schedules/{id}/occurrences` is the history, newest first.

## Ticket title placeholders

Four substitutions are available in the schedule's ticket template, and deliberately only four — enough to make each occurrence nameable on the board, not a template language.

| Placeholder | Renders | Zone |
|---|---|---|
| `{date}` | `2026-08-03` | the cadence's `tz` |
| `{week}` | `2026-W32` (ISO week) | the cadence's `tz` |
| `{month}` | `2026-08` | the cadence's `tz` |
| `{slot}` | `2026-08-03T07:00:00Z` | UTC |

The first three are labels for a human, so they are local: a ticket for 23:30 in Berlin must not be named for tomorrow. `{slot}` is the occurrence's identity, so it is the UTC instant.
