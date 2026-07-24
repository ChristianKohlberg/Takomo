# Promotions

A **promotion** records that a ticket's work reached some named target or stage
— and it's deliberately not tied to software. `target` is a free-form string, so
it fits whatever your team ships: `staging` / `production` for a deploy,
`draft` → `published` for content, `delivered` for a client hand-off, `printed`,
`rolled-out`, anything.

Promotions are **append-only history** and live alongside — not inside — the
workflow state machine. A ticket can be `done` and then promoted to several
targets over time; promoting never changes the ticket's state.

## Recording one

```sh
takomo promote TKM-482 production --url https://deploy/run/91 --ref v1.4.0 --note "canary then full"
takomo promote TKM-482 staging
```

Over the API / MCP:

```
POST /v1/tickets/{id}/promote   { "target": "production", "url": "...", "ref": "...", "note": "..." }   # write scope
takomo_promote { id, target, url?, ref?, note? }
```

Only `target` is required (1–100 chars); `url`, `ref`, and `note` are optional.

## Seeing them

- **Board card** — the latest promotion shows as a quiet badge, e.g.
  `● production · 2h`. The board fetches the latest-per-ticket in one call
  (`GET /v1/promotions?project=<id>`), so a heavy board stays cheap.
- **Detail drawer** — the full history, newest first
  (`GET /v1/tickets/{id}?include=promotions`, or
  `GET /v1/tickets/{id}/promotions`).
- **Agents** — `takomo_show` includes a ticket's promotions, and every promotion
  emits a `ticket_promoted` event on the log.
