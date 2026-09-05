# Mindmaps: thinking out loud, before it is anything

Takomo has a home for work (tickets), for an idea being nurtured (initiatives),
and for verification (checks). It had nothing for the ten minutes *before* any of
those — the conversation where a project idea fans out into API, integrations,
workflows, ideas, six words a thought, and a branch splits in two the moment it
turns out to be two thoughts.

A **mindmap** is that surface: a project-scoped tree you grow at conversation
speed, whose branches can graduate into epics and initiatives, and which then
keeps the link — so a map that mattered stays a way to navigate what it became.

## Several people at once

A brainstorm is a conversation, and a conversation has more than one person in
it. So a map is not rows — it is **one CRDT document**, the same machinery
`/documents` runs, and everyone on it is an ordinary peer: two people typing and
an agent adding a branch all write to the same replica and see each other
without reloading. There is no save button and no dirty state, because the
question "did my change save" is replaced by "am I connected".

`spec/mindmap-crdt.md` is the design; `src/store/mindmapdoc.rs` is the document
model. Three consequences are worth knowing:

**A move merges; a cycle is repaired.** A node carries a pointer to its parent,
so dragging it somewhere is one field write rather than a delete and an insert
(which is what loses a subtree when two people drag at once). The price is that
two people can each make a legal move that together form a loop — neither was
wrong, and no check at write time could have seen it coming. So the tree is
repaired **when it is read**, by a fixed rule: the member of the loop with the
lowest id comes back to the root. Every reader computes the same tree, which is
the only property that matters. A node whose parent was pruned underneath it
comes back to the root too, rather than disappearing.

**Order is a fractional index, not a number.** Two people inserting between the
same pair would both pick the same gapped integer. The stored key is a string
with a midpoint between any two others, so an insert never renumbers anything
and never has to agree with anybody. The API still reports `position` as a plain
rank among siblings, because that is what it always meant to a reader.

**The caps are enforced where writes are checked.** The server sees an API call;
it does not see an individual keystroke arriving over the sync socket. So every
cap below holds on REST, MCP and the CLI — which is where agents write, and
agents are what would blow them — and the editor holds them in the browser. This
is the same trust model `/documents` already runs.

## What it deliberately is not

A brainstorming method, nothing more. No workflow, no claim, no lease, no ready
queue, no assignment, no comments, no expiry.

Two rules follow, and they shape everything:

**Deleting one is ordinary.** An initiative earns the right to be nurtured; a
mindmap earns the right to be thrown away. `takomo mindmap rm` cascades its nodes
and touches nothing its branches became — those graduated and are work in their
own right. That is what makes it safe to start a map early, which is the only
time a brainstorm is worth anything.

**A node's title is capped at 280 characters**, and that is the method rather
than a limitation. A brainstorm whose nodes grow into paragraphs has quietly
become a document, and the thing that made it valuable — reading a whole branch
at a glance — is gone by the time anyone notices.

The rule has not been retired, it has been **relocated**. A node now also has
`notes`, which do not render in the outline and are read by opening the node. So
brevity is enforced exactly where it does the work — the line you scan — and
detail no longer has to be truncated or promoted away:

```
That title is 300 characters and the cap is 280. A mindmap node is a sentence or
two — that brevity is what makes a branch readable at a glance.
remedy: Shorten it, move the detail into the node's notes, split it into two
nodes, or promote the branch to an initiative where the long form belongs.
```

## One per project

A project holds one brainstorm, the way it holds one board. "Which map?" is not a
question anybody wanted to answer, and a rail of half-started maps is how a
surface stops being used — so starting a second is refused, and the refusal names
the one that exists, because that is almost always the one you wanted:

```
Project 'tp' already has a mindmap (mm-1cslpg34). A project has one brainstorm,
the way it has one board.
remedy: Grow that one instead, promote the branch that turned out to be its own
subject into an initiative or an epic, or throw the map away first — deleting one
is ordinary.
```

The branch that has become its own subject is what promotion is for, and it was
always the way out.

**The data model is untouched.** `mindmaps` is still a table keyed by project,
every read still filters and pages, and a project that already holds more than
one keeps them, lists them and can open them — nothing deletes somebody's
thinking to enforce a new rule. Allowing several again is deleting a check, not
running a migration.

## Growing one

```sh
takomo mindmap new "Payments rebuild"          # the title is the root
takomo mindmap add mm-1cslpg34 "API"
takomo mindmap add mm-1cslpg34 "versioning: v1 forever, or dated?" --parent mn-hr9hcrgp
takomo mindmap show mm-1cslpg34
```

```
# Payments rebuild   (mm-1cslpg34, open)
- API   [mn-hr9hcrgp]
  - versioning: v1 forever, or dated?   [mn-54tvdp1u]
  - idempotent retries on capture       [mn-jj6tyi3x]
- integrations   [mn-c3gftogt]
- workflows      [mn-r4bk7rc9]
- ideas          [mn-rrnen6n5]
```

The node id sits beside each line because the next thing anyone does is hang a
thought under one of them, and that takes an id.

### With an agent, which is the other half

Brainstorming with Claude means the agent adds a whole branch while you talk, so
the node route takes a **batch** and `takomo_mindmap_grow` is built around it:

```json
{ "id": "mm-1cslpg34", "nodes": [
  { "text": "API" }, { "text": "integrations" }, { "text": "workflows" }
] }
```

Fifty nodes per call, and the batch lands whole or not at all — half a branch
arriving would leave a map nobody asked for and no way to tell which half.
`takomo_mindmap_show` reads it back as the indented text above, which is the
cheapest shape for a model to reason about, with the ids alongside.

## What a branch becomes

```sh
takomo mindmap promote mm-1cslpg34 mn-hr9hcrgp --to epic
takomo mindmap promote mm-1cslpg34 mn-c3gftogt --to initiative
```

| target | makes | for |
|---|---|---|
| `epic` | an epic titled from the node, with its **direct children** as tickets under it | the fastest path from "we talked about it" to work in the queue |
| `initiative` | an initiative titled from the node, with the subtree as its first **entry** | a direction that needs nurturing before it is work |

Only direct children become tickets. A deeper subtree would arrive as a flat pile
whose shape nobody could recover — and the map keeps that shape for whoever wants
it. The initiative gets the subtree as an *entry* rather than a summary because an
entry carries provenance (`source: mindmap:mm-…`), and where an idea came from is
exactly what a collection is supposed to remember.

**The node stays**, carrying what it became:

```
- API   [mn-hr9hcrgp]  -> epic demo-ziuq
- integrations   [mn-c3gftogt]  -> initiative ini-dx161vhm
```

Promotion is not a move. The map is the record of how the thinking got there, and
a branch that vanished the moment it mattered would make the map worthless as the
thing you read afterwards. This is also the answer to "can it stay longer as a
navigation tool": a map that produced work becomes a picture of that work.

Promoting the same branch twice is refused — it would make a second epic from one
thought, indistinguishable from the first.

## Caps, and why each one exists

| cap | value | why |
|---|---|---|
| node title | 280 chars | the method, above |
| node notes | 8,000 chars | the long form of ONE thought; past it the branch wants to be an initiative |
| relationships per map | 1,000 | a canvas, not a graph database |
| attachments per node | 20 | pointers are cheap, but a node with fifty of them is a folder |
| nodes per map | 500 | a brainstorm, not a database. Past it: promote branches, or start a second map |
| depth | 8 | the ceiling the initiative folder tree uses — past it nobody can read the shape |
| nodes per call | 50 | one agent turn. The loop runs inside the write transaction that serializes every claim in the store |

The whole map comes back in one read (`GET /v1/mindmaps/{id}`) precisely because
of the 500 cap: a canvas cannot draw half a tree, and paging one would be a
worse contract than bounding it.

**Where the caps hold.** They are enforced on the REST and MCP path, and the
canvas keeps them as it writes. They are not enforced on the sync socket, and
cannot be: a CRDT update applies whole or not at all, so refusing one for
carrying the 501st node would desynchronise that peer rather than teach it
anything. A peer that ignores them can therefore exceed them — measured at 713
nodes and a 5,000-character title. That is the honest position: these caps are
the brainstorm discipline this page describes, kept by the surfaces people write
through, not a boundary the server can hold against a determined client. What the
socket does bound is the resource — one message is capped, so no single write can
sit on the store's write mutex.

## Placement, order and shape

`position` on the wire is the node's **rank among its siblings** — 0, 1, 2. The
key actually stored is a fractional index (see above); it never leaves the
server, because it is an opaque string that has to stay free to change shape.
Writing `position` means "put it at this index in its ring".

`at` is hand placement — `{x, y}` or `null` to let the layout place it. Null by
default, which keeps a map growing at typing speed tidy; a map that *has* been
arranged stays exactly where it was left, and `at: null` hands one node back to
the layout ("tidy up" clears them all). The pair is one field rather than two
nullable numbers, because half a coordinate places nothing.

Hanging a node off its own descendant is refused (`mindmap.cycle`). It would cut
that branch off the map, and it is exactly what a drag makes easy to try. Two
people doing it *simultaneously* cannot be refused — see the repair rule above.

## Relationships — the edge that is not the tree

The hierarchy answers "what is this part of". Everything else is a
**relationship**: `{from, to, label}`, outside the tree, drawn differently on the
canvas. A question hanging off the thing it questions, a screen that navigates to
another, a plain "see also" — one mechanism instead of three special cases.

```sh
POST   /v1/mindmaps/{id}/relationships          {"from":"mn-…","to":"mn-…","label":"answers"}
DELETE /v1/mindmaps/{id}/relationships/{rel}
```

A relationship whose end no longer resolves is dropped when the map is read,
never repaired: there is no node to point at, and half an edge is not a fact
about anything. Pruning a branch takes its relationships with it.

## The map and the document are one plan

A project has **one plan**, and two ways of looking at it. The map is for
thinking — grouping topics fast, before anybody knows what they are. The
document view at `/documents` is for writing them out.

They are not two things kept in step. **A node IS a section**: its title is the
heading, its depth is the heading level, tree order is reading order, and its
prose lives inside the node itself. Both views write the same document, so they
cannot drift, and two people can be in different views of the same thought at
the same time.

There is nothing to convert and nothing to re-run.

```
the map                          the document view
─────────                        ─────────────────
API                              1    API
  versioning?                    1.1    versioning?
integrations                     2    integrations
```

### Where each section stands

`GET /v1/mindmaps/{id}` carries `standing` per section: when it last changed,
when it was last confirmed, and whether that confirmation still holds. It is a
**reading**, not a stored flag — a section confirmed before its last edit is not
confirmed any more, and no boolean can say that.

### The trace

`GET /v1/mindmaps/{id}/trace` is what happened to the plan and who did it:
`authored`, `renamed`, `edited`, `moved`, `pruned`, `reviewed`, `proposed`,
`accepted`, `rejected`. Every entry names the person behind the credential, not
just the actor string, because a scope is not an identity.

It is sparse on purpose — an act somebody would name, never a keystroke — and
the acts that changed the prose keep what it then said, which is what a diff is
made of. The CRDT update log cannot answer that: compaction rewrites it into one
blob by design.

A caller may record only `edited`, `reviewed`, `accepted` and `rejected` — the
four the server cannot observe. It records what it performs itself, so nobody
can claim to have moved a node they did not move.

### An agent proposes; a person accepts

Unchanged, and only re-aimed. An agent reads a section annotated with block ids
(`takomo_plan_read`) and replies with OPERATIONS against those ids
(`takomo_plan_propose`) — never with a document, which is what keeps somebody's
concurrent typing. Nothing is live until a person accepts it in the document
view.

## Attachments — a pointer, never the file

A node can point at something that lives elsewhere: `{kind, name, gist, ref}`,
where `kind` is one of `pdf`, `code`, `table`, `diagram`, `audio`, `link`.

**Never the bytes.** Initiative entries are the only place in this store that
holds binary blobs, and they carry byte caps because an unbounded upload holds
the write mutex every claim waits on. Here the argument is stronger: bytes in a
CRDT log are replayed by every peer that joins, so one PDF would make the map
slower to open for everybody, forever.

## What reaches the event log

`mindmap_created`, `mindmap_grown` (one event per batch, not per node — ten nodes
from an agent turn are one act of brainstorming), `mindmap_moved` (a reparent
only), `mindmap_pruned`, `mindmap_promoted`, `mindmap_updated`, `mindmap_deleted`.

Text and placement edits reach nothing. They change constantly while somebody is
thinking, and an event per keystroke-batch would bury every other event in the
project under one person's typing. Nothing typed over the sync socket emits an
event at all, for the same reason — and no event is added for relationships,
because one created in the browser would emit nothing while one created over the
API would emit something, and an event that fires half the time is worse than no
event.

## Surfaces

- **CLI** — `takomo mindmap new|ls|show|add|promote|rm`
- **MCP** — `takomo_mindmap_new`, `takomo_mindmap_grow`, `takomo_mindmap_show`,
  `takomo_mindmap_list`, `takomo_mindmap_promote`. The two reads are free.
- **REST** — `/v1/mindmaps`, see `spec/openapi.yaml`
- **A canvas** at `/mindmaps`, and the canvas is the whole page. There is no
  list to choose from — a project holds one brainstorm, so there was never a
  choice to make, and a rail listing one thing is chrome pretending to be
  navigation. Live and shared, showing who else is on it. Drag a node onto
  another to reparent it, into space to pin it; Enter for the next thought, Tab
  to go deeper, F2 to retype, Delete to prune (a branch with children asks
  twice, because everyone is watching it go).

  **A node carries its own detail.** Selecting one grows its card in place —
  title, notes, kind, colour, shape, edge label, attachments, the relations
  touching it. Every other node stays a line with marks (`≋` notes, `¶` things
  attached, `→ epic` promoted), so a map you have not opened still shows where
  the substance is. Nothing lives in a panel at the edge of the screen, because
  the map is the thing you are working in.

  **⌘K is how you reach the rest**, scoped to the selected node or to the map
  and saying which. Add, rename, write notes, relate, attach, promote, fold,
  prune; fit, tidy, rename the map, switch project. And **go to a thought…**,
  which fuzzy-matches titles, unfolds whatever was hiding the match and centres
  it — with no rail, that is how you move around a map that has outgrown the
  screen. A node with no hand placement is laid out automatically, which
  keeps a map growing at typing speed tidy, and "tidy up" hands every pinned node
  back to the layout. On a phone the same tree is an indented list — a better
  shape for the screen than a pinch-zoom canvas, not a consolation prize.

## Dictation

**A brainstorm arrives faster than it can be typed.** The 280-character node cap exists precisely
because a thought that size is one sentence somebody said, so the map takes dictation: press
**Dictate** in the action bar, talk, and every finished sentence lands as a thought. They grow one
branch under whatever was selected when the microphone opened — fixed at that moment rather than
read per sentence, because following the selection would deepen a chain a node per sentence, which
is not what a spoken list is. A partial transcript is shown and thrown away; only a finished turn
becomes a node.

**The audio never reaches this server.** The page streams it straight to AssemblyAI over a
WebSocket, which keeps a recording out of the request log and off the disk. What the page must not
hold is the account key — a key in a page is a key on every machine that loads it — so
`POST /v1/speech/token` exchanges it for a token good for ten minutes and one session. That route
takes `write`, not `read`: dictation exists to change a map, and it costs the operator money per
minute.

**Off unless `TAKOMO_ASSEMBLYAI_API_KEY` is set.** `/v1/whoami` reports `features.voice`, so the map
leaves the button out rather than offering one that answers `503 speech.not_configured` — the same
rule `/documents` follows for its prompt bar.

### Collaboration lifecycle and navigation

The map and document view allocate their shared replica and socket after a
committed React render, so StrictMode replay cannot destroy the active editor.
Cross-tab broadcast-channel sync is disabled: each tab uses its scoped WebSocket
and server authorization. Expiring tickets renew without replacing the replica.
A peer that misses the broadcast buffer receives a full CRDT update to recover;
corrupt stored updates fail opening rather than being silently discarded.

The view switcher remembers URLs per project and keeps native link behavior.
Map selection follows browser history. Its compact controls have accessible
names on phones. Project change notifications refresh metadata and lists while
content continues to arrive through the existing CRDT connection.

### Project navigation and local recovery

The plan views use `?project=<id>` for explicit project scope and `#n=<node>` for
personal section selection. The shared Document / Map / Tests switch carries
both. Changing project clears its editor/filter selection; browser Back restores
the earlier scope and section. A legacy `#m=` map link is resolved to the map's
actual project. Selection does not move other collaborators' viewports.

Editable map/document replicas and check definitions append their CRDT updates
to IndexedDB, partitioned by server origin, actor, and object. Both plan views
use the same log. Log compaction merges stored updates transactionally rather
than overwriting another tab's unsent edits with a local snapshot. Navigation
allows queued writes to finish. Restoring a replica requires a newly authorized
session; a read-only session does not replay a writer's cached edits.

`Saved` means the server acknowledged a persistence barrier covering this
browser's preceding updates and its local writes have completed. `Syncing`
means that acknowledgement is outstanding. `Offline · saved on this device`
means local storage completed while the socket is disconnected. Storage errors
and server refusals have separate visible states; neither claims success.

The binary WebSocket extension is opt-in via `durability_ack` in the session
response: message type **4**, followed by an unsigned sequence number, requests
an ordered durability barrier. The reply carries **4**, the same sequence, and
**1** for persisted or **0** for refused/pending. The barrier rechecks the session
and serializes with the room flusher. It includes deletions, which cannot be
acknowledged using a state vector alone. Older Yjs sync/awareness frames remain
unchanged. Project notification sockets do not support this extension.

Local recovery protects edits across reloads and reconnects. It does not install
an offline application shell: opening/reloading the whole site without network
access may require reconnecting before the editor can load and obtain a session.
Clearing browser site data removes local drafts. Server-authorized verdicts,
approvals, and configuration actions remain online operations.
