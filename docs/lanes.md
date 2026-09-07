# Lanes and review handoffs

A lane collects related tickets and retains the context used to prepare and resume
work. Tickets remain the source records. A handoff captures an explicit assignment;
there is no additional batch container.

Open **Lanes** in the navigation rail, create a lane with a purpose, and add related
tickets. Record durable context: decisions, constraints, useful source links, and
unresolved questions. An organizing agent can do the same through the CLI, native
MCP, or REST API. Lane names and ticket associations are project-defined; lanes
are not automatically generated from specification headings.

## Organize pending work

Choose **Organize pending work** on the Lanes page to ask the existing Codex
app-server service for a proposal. Give it any grouping priorities or constraints.
It considers pending tickets without an active lane, existing lanes and their
durable context, and saved project specification text. It proposes new or existing
lanes, explains each grouping, prepares context, and identifies questions that
prevent the work from being ready. Unclear or potentially duplicate work can stay
unassigned with an explanation. No predefined lane names or project structure are
imposed.

Review the proposal before accepting it. Acceptance applies the complete grouping
and prepared context together; it does not create or send an implementation
assignment. Existing lane names and purposes are preserved. If the source work
changed while the proposal was being prepared or reviewed, request a fresh
proposal. You can give revised instructions before requesting another proposal,
or edit lanes normally after acceptance.

The first version reads persisted specifications; unsaved editor changes are not
included. Requests are limited to 200 pending tickets, 100 active lanes and 20
specifications, with a combined 512,000-byte snapshot limit. Oversized projects
receive an error instead of silently organizing a partial collection. Saving or
changing included source material before acceptance requires a fresh proposal.

The organizer uses `services/agent`, with the same dedicated Codex authentication,
persistent service state and project-scoped `agent:run` token as section
conversations. Update and restart that service when adopting this feature. Without
a running service, requests remain queued. The organizer has no repository or
write tools; Takomo validates its structured answer and applies accepted changes.
Readiness is advice, not proof that implementation or review has succeeded.

For work already selected within a lane, a preparation handoff can enrich that
assignment through the separate lane worker. Agent grouping can also be performed
through the ordinary lane and ticket tools.

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
