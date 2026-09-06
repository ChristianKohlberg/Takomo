# Lanes and review handoffs

A lane collects related tickets and retains the context used to prepare and resume
work. Tickets remain the source records. A handoff captures an explicit assignment;
there is no additional batch container.

Open **Lanes** in the navigation rail, create a lane with a purpose, and add related
tickets. Record durable context: decisions, constraints, useful source links, and
unresolved questions. An organizing agent can do the same through the CLI, native
MCP, or REST API. Lane names and ticket associations are project-defined; lanes
are not automatically generated from specification headings.

Create a preparation handoff to have an agent enrich the selected work. Preparation
produces context for a later assignment. It does not automatically dispatch
implementation. Agent grouping can also be performed through the ordinary lane
and ticket tools, without running an embedded agent.

## Dispatch and execution

A draft fixes the selected ticket contents and lane context at creation. Inspect
that scope before choosing **Send**. Creating or editing a lane, adding a ticket,
and drafting a handoff do not execute an agent. Dispatch requires a credential
with `write` and `human` (or admin authority).

Codex and Claude execution is performed by a separately configured local lane
worker. Without that worker, a dispatched assignment remains queued; the server
does not run a model or access a checkout itself. See the worker's README under
`services/lanes` for setup, workspace isolation, and provider requirements.

Assignments keep their own status and result. A handoff completing does not
silently close its tickets or its lane. New lane tickets do not change an existing
assignment. Cancel and create a new draft when the intended scope changes.

## Review and correction

After implementation, create a review handoff for the completed implementation
and its exact result revision. The review uses independent context and returns
its findings to the original lane. It never implies approval of later revisions.
Use those findings when preparing a correction assignment, then review the new
implementation revision. Earlier results remain readable throughout the cycle.

Durable lane context is available even when a provider conversation cannot be
resumed. Provider conversations are an optimization, not the sole record of the
assignment. A preparation result must not silently replace lane context that was
edited while the preparation ran.

## Agent access

Native and stdio MCP expose `takomo_lanes`, `takomo_lane_show`,
`takomo_lane_create`, `takomo_lane_update`, `takomo_lane_ticket`,
`takomo_lane_handoff`, and `takomo_lane_handoffs`. These tools organize work and
create drafts; they do not dispatch execution.

```sh
takomo lane new "Offline editing" --project demo
takomo lane add LANE_ID TICKET_ID
takomo lane show LANE_ID
takomo lane set LANE_ID --file context.json
takomo lane handoff LANE_ID --file assignment.json
takomo handoff show HANDOFF_ID
# Only after explicitly deciding to execute this assignment:
takomo handoff send HANDOFF_ID
```

`context.json` contains fields such as `{"context":"Decisions and constraints"}`.
An implementation assignment contains `kind`, `provider`, `instructions` and
`ticket_ids`; a review also requires `parent_handoff` and `target_revision`.
`--file -` reads JSON from stdin. Commands return JSON, including IDs and history.

Existing epics and initiatives remain available. This feature introduces the lane
flow without guessing how existing records should migrate. Removing or converting
those records is a separate product decision. Bugs remain normal tickets; a focused
Bugs page can coexist with lanes.

## Rollback

The storage change is additive. Stop lane workers and cancel queued work before
rolling back the application. Preserve the database and local worker state;
existing tickets, epics and initiatives are not rewritten by this feature. A
failed execution may leave work in its explicitly mapped checkout for inspection;
no automatic cleanup should discard that work.
