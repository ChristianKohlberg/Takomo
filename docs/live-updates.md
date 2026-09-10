# Project live updates

The existing project WebSocket sends `{"type":"refresh"}` on connection and after
lost broadcast history. This requests a complete resync, including for older
clients. Normal committed changes add a `topics` array: `document`, `trace`,
`checks`, `projects`, `inbox`, `tickets`, `agent`, `history`, or `search`.
Clients without topic support can continue refreshing on every message.

Notifications are collected centrally by connection-local SQLite triggers on the
single writer, then published only after a successful transaction commit. They
contain project IDs and topic names, never row contents or credentials. Updates
capture both old and new ownership. Duplicate topics coalesce per transaction and
the socket batches relevant changes for 400 ms. Rollbacks do not notify.

`src/store/live_updates.rs` is the explicit inventory. Document changes also
invalidate ticket reference projections; check changes invalidate document check
summaries. Ticket, dependency and workflow changes conservatively invalidate the
tickets topic globally because blockers can belong to another project. Project
catalog changes notify every project's catalog; authorization changes can request
a full resync. Unknown future tables conservatively request a full resync until
classified. Foreign-key deletion rules preserve the owning parent's topics
without broadcasting orphaned child lookups to unrelated projects.

Token/share/grant usage timestamps, agent lease-only heartbeats, session and OAuth
bookkeeping, idempotency records, search index caches and internal outboxes do not
invalidate document lists. Actual token revocation or permission changes still
notify. Search status changes use their own topic. The older internal store watch
is retained, but project sockets no longer subscribe to it.

Trace is loaded for an opened section history, not every document refresh. The
inbox uses live `inbox` notifications while connected, with polling recovery when
the socket is unavailable. Initial connection and reconnection resync prevent
missed offline changes. These changes reduce unrelated requests; they do not
promise that an open page makes no background requests.
