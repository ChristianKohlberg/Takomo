# Mindmaps as a collaborative object

Status: spec for implementation · replaces the relational mindmap in place

A mindmap stops being rows and becomes a CRDT, so two people (and an agent) can
grow one at the same time. This is the same machinery `/documents` already runs,
generalised rather than duplicated: one room layer, one update log, one sync
credential, serving both kinds of object.

The read contract does not change. `GET /v1/mindmaps/{id}` returns the same JSON
it returns today, with new fields added — so every MCP tool, the CLI, and the
canvas keep working while the storage underneath is replaced.

---

## 1. What changes, in one table

| | before | after |
|---|---|---|
| node storage | `mindmap_nodes` rows | a Y.Doc per mindmap |
| map metadata | `mindmaps` row | unchanged |
| sibling order | gapped integer `position` | fractional index string, integer rank exposed |
| cross-links | none | `Relationship`, outside the hierarchy |
| node text | `text`, 280 cap | `title`, 280 cap — plus uncapped-in-spirit `notes` |
| fold state | none | per-viewer, browser-local, never in the CRDT |
| writes | one SQL transaction each | room mutations, broadcast live |
| update log | `doc_updates` (documents only) | `crdt_updates` (both kinds) |
| sync credential | `doc_sessions` (documents only) | `crdt_sessions` (both kinds) |

---

## 2. The generalisation

`src/api/docsync.rs` is already almost object-agnostic. `Room` is keyed by a
string id, hydrated through `load_doc_updates(id)`, flushed through
`append_doc_update(id, …)`. Only those two store calls and two table foreign
keys tie it to documents.

**Object ids already carry their own kind.** `src/ids.rs` mints `doc-…`, `mm-…`,
`mn-…`. So the room key stays a bare object id and the loader dispatches on the
prefix. Nothing needs a composite key, which matters for one specific reason
given below.

### 2.1 Tables

Two renames, one new column pair on each.

```sql
CREATE TABLE crdt_updates (
  seq         INTEGER PRIMARY KEY AUTOINCREMENT,
  object_kind TEXT NOT NULL,          -- 'document' | 'mindmap'
  object_id   TEXT NOT NULL,
  blob        BLOB NOT NULL,
  bytes       INTEGER NOT NULL,
  created_by  TEXT NOT NULL,
  created_at  INTEGER NOT NULL
);
CREATE INDEX idx_crdt_updates_object ON crdt_updates(object_id, seq);

CREATE TABLE crdt_sessions (
  id          TEXT PRIMARY KEY,
  token_hash  TEXT NOT NULL UNIQUE,
  object_kind TEXT NOT NULL,
  object_id   TEXT NOT NULL,
  project     TEXT NOT NULL,
  actor       TEXT NOT NULL,
  "user"      TEXT REFERENCES users(id),
  display     TEXT NOT NULL,
  can_write   INTEGER NOT NULL,
  expires_at  INTEGER NOT NULL,
  created_at  INTEGER NOT NULL,
  revoked_at  INTEGER
);
CREATE INDEX idx_crdt_sessions_object ON crdt_sessions(object_id);
```

There is **no foreign key to the owning object**, and that is deliberate: the
column now points at one of two tables. Ownership is validated in Rust — the same
call the repo already makes for `checks.epic`, `checks.initiative` and
`documents.initiative`, and for the same reason (a teaching 422 instead of an
opaque FOREIGN KEY failure). The cascade the FK used to provide is replaced by an
explicit delete of an object's updates and sessions in the delete path.

### 2.2 The migration trap

`rename_lanes_to_checks` in `src/store/mod.rs` runs **before** the schema batch,
because `CREATE TABLE IF NOT EXISTS checks` would otherwise create an empty table
beside the populated one. This migration has exactly that shape and must follow
exactly that precedent:

1. run before the `CREATE TABLE IF NOT EXISTS` batch
2. if `doc_updates` exists and `crdt_updates` does not: create, copy every row
   with `object_kind = 'document'`, drop the old table
3. same for `doc_sessions` → `crdt_sessions`
4. only then let the schema batch run

Get the order wrong and every existing document loses its history in a way that
looks like success.

### 2.3 Sync route

`/v1/docsync/{id}` stays reachable, and a `/v1/sync/{id}` alias is added with the
old path kept so open browser sessions do not break.

**The id must remain the last path segment.** This is a wire requirement, not a
preference: `y-websocket` builds its URL as `serverUrl + "/" + room + "?" +
params`, so anything after the room — or a base already carrying a query string —
comes out mangled. `/v1/documents/{id}/sync` did exactly that and was reverted.
The same rule binds the mindmap route, and it is why the room key is a bare id
rather than a `kind:id` pair.

Session mint gets a sibling: `POST /v1/mindmaps/{id}/session`, alongside the
existing `POST /v1/documents/{id}/session`. Both write `crdt_sessions`.

The `tkd_` prefix is **kept and widened** from "document session" to "collab
session". It is not a new credential kind: same lifetime, same revocation, same
`can_write` copied from the minting token's scopes at mint time, same reason for
existing (a browser WebSocket cannot set an `Authorization` header). A sixth
prefix would imply a sixth auth path, and there is not one.

---

## 3. The Y.Doc

Two top-level maps. Both keyed by id, neither an array — an array makes removal
fight an insert, and keyed maps merge cleanly.

```
nodes:         Y.Map<nodeId, Y.Map>
relationships: Y.Map<relId,  Y.Map>
```

### 3.1 Node

| field | type | notes |
|---|---|---|
| `parent` | string \| null | `null` = root. Last-write-wins. |
| `order` | string | fractional index; see §4 |
| `title` | Y.Text | 280 cap. Character-level merge. |
| `notes` | Y.Text | the long form; see §6 |
| `x`, `y` | number \| null | hand placement. Both null or both set. |
| `edge_label` | string | labels the edge **to this node's parent** |
| `kind` | string | `thought` (default) \| `question` \| `decision` \| `screen` \| `component` |
| `origin` | string | `human` \| `agent` |
| `reviewed` | boolean | an agent node a person has looked at |
| `icons` | Y.Array\<string\> | short tokens, max 8 |
| `color`, `shape` | string | explicit fields, **not** a style blob |
| `attachments` | Y.Map\<attId, Y.Map\> | see §3.3 |
| `promoted_kind` | string \| null | `epic` \| `initiative` |
| `promoted_id` | string \| null | the thing it became |

`title` and `notes` are `Y.Text` because two people do type into the same node —
that is the collaborative case, and last-write-wins would silently discard one of
them. Everything else is a scalar and merges last-write-wins, which is correct
for a colour or a coordinate.

**`edge_label` on the node is unambiguous** because a node has exactly one parent.
It describes the hierarchy edge; `Relationship.label` describes everything else.

**`style` is not a JSON blob.** A blob merges as a whole, so two people changing
different things about one node clobber each other. Explicit fields merge
independently. More fields can be added later; a blob cannot be un-blobbed.

### 3.2 Relationship

An edge that is **not** part of the hierarchy.

| field | type |
|---|---|
| `from` | node id |
| `to` | node id |
| `label` | string |

This is what carries the prototype's question-node links, its cross-references,
and any "see also" edge. One mechanism instead of three special cases.

A relationship whose `from` or `to` no longer resolves is **dropped on read**, not
repaired — a dangling edge is not a node and there is nothing to keep.

Relationships do not affect the tree, do not participate in cycle detection, and
do not count against the node cap. They have their own cap: 1,000 per map.

### 3.3 Attachments

An attachment is a **pointer, never a blob**:

| field | type |
|---|---|
| `kind` | `pdf` \| `code` \| `table` \| `diagram` \| `audio` \| `link` |
| `name` | string, max 200 |
| `gist` | string, max 500 — one line of what it says |
| `ref` | string, max 2000 — a URL or path |

Initiative entries are the only place in this store that hold binary blobs, and
they carry byte caps for a reason: an unbounded upload holds the write mutex
every claim waits on. That argument is *stronger* here, because the bytes would
land in a CRDT update log that every peer replays on join. So the mindmap stores
where a thing is, not the thing. Max 20 attachments per node.

---

## 4. Ordering

Gapped integers cannot survive concurrency — two peers inserting between the same
pair both pick 1500.

`order` is a **fractional index**: a base62 string with a `between(a, b)`
midpoint. Siblings sort by `order`, ties broken by node id so the result is
total and identical on every peer.

**Both sides assign it**, because nodes are created at typing speed in the
browser (Enter, Tab) and in batches by an agent through the API. That means two
implementations — `src/fracdex.rs` and `web/src/lib/fracdex.ts` — and they must
not drift. They are held together by a **shared test-vector file** checked by both
suites: same inputs, same outputs, byte for byte. A crewmate that implements one
without the other, or without the vectors, has not finished.

**The API never exposes the order key.** `GET` returns `position` as an integer
sibling rank (0, 1, 2…) derived from the sorted order, exactly as it reads today.
Writes accept `after: <node id>` or an index. The contract stays what
`spec/openapi.yaml` already says; only the storage changed.

---

## 5. Reading the tree

A parent pointer under concurrency admits a state no synchronous validator could
have produced: two people each make a legal move that together form a cycle — A
under B while B goes under A. The store refuses this today and cannot any more.

So the tree is **normalised deterministically on read**, and the rule is fixed so
every peer computes the same tree from the same state:

1. **Cycles** — walk each node's ancestry. A node whose ancestry loops is
   re-parented to root. Where a cycle has several members, the one with the
   lowest node id is the one that moves, so the choice does not depend on
   traversal order.
2. **Orphans** — a node whose `parent` names a node that no longer exists is
   re-parented to root, not dropped. Losing a subtree because its parent was
   deleted concurrently is worse than showing it.
3. **Depth** — the depth-8 ceiling is not enforced on read. A map that exceeds it
   renders; the API refuses to *create* past it. Silently moving somebody's node
   because a branch got deep is a worse outcome than a deep branch.

Normalisation is a pure function over the node set, in `src/store/mindmaps.rs`,
with unit tests — the same place the current depth and cycle checks live. The
browser applies the identical rule when rendering.

---

## 6. Caps, and the one rule that changes

| cap | value | enforced |
|---|---|---|
| `title` | 280 chars | API + editor |
| `notes` | 8,000 chars | API + editor |
| nodes per map | 500 | API + editor |
| relationships per map | 1,000 | API + editor |
| attachments per node | 20 | API + editor |
| depth | 8 | API only (see §5) |
| nodes per batch | 50 | API — unchanged |

**What "enforced on the API" now means.** The server no longer sees an individual
keystroke: a browser writes over the sync socket and the server only ever sees a
merged update. This is the trust model `/documents` already runs — nothing
validates a document's prose either — and it is bounded the same way: the socket
credential reaches exactly one object, expires, and is revocable.

So the caps hold on every REST, MCP and CLI write, which is where agents live and
agents are what would blow them, and the editor holds them in the browser. What is
lost is server enforcement against a person typing into a box. That is a real
loss and it is accepted knowingly.

### The 280-character rule, restated

Today the cap is described as the method: a node that outgrows a sentence has
become a document, and reading a branch at a glance is what made it valuable.

Adding `notes` does not retire that argument — it **relocates** it. Brevity is
enforced where it does the work: the `title` is what the outline and the canvas
render, and it stays capped at 280 with the refusal that names the way out. `notes`
is the escape hatch, does not appear in the outline, and is revealed on a node the
reader has opened.

The teaching refusal keeps its wording for `title`, minus the promise that the
only way out is promotion:

```
That title is 300 characters and the cap is 280. A mindmap node is a sentence or
two — that brevity is what makes a branch readable at a glance.
remedy: Shorten it, move the detail into the node's notes, split it into two
nodes, or promote the branch to an initiative where the long form belongs.
```

---

## 7. Writes

Every mutating path becomes a room mutation, following the pattern
`src/api/docs.rs:247` and `src/mcp.rs` already use: `open_room(&state, &id)`, then
`room.mutate(|doc| …)`, which applies the change to the one authoritative replica
and broadcasts the resulting update to every connected peer.

The consequence is worth naming because it is the point of the whole change:
**an agent growing a branch appears in an open browser immediately**, rather than
on the next reload. The agent and the people looking at the map are on one
replica, not two that have to be reconciled.

Routes, all preserved:

| route | change |
|---|---|
| `GET /v1/mindmaps` | none — reads the `mindmaps` rows |
| `POST /v1/mindmaps` | creates the row and an empty Y.Doc |
| `GET /v1/mindmaps/{id}` | reads the room; same JSON, new fields |
| `GET /v1/mindmaps/{id}/outline` | reads the room |
| `PATCH /v1/mindmaps/{id}` | none — row metadata |
| `DELETE /v1/mindmaps/{id}` | also deletes updates and sessions (§2.1) |
| `POST /v1/mindmaps/{id}/nodes` | room mutation, batch of 50 |
| `PATCH /v1/mindmaps/{id}/nodes/{node}` | room mutation |
| `DELETE /v1/mindmaps/{id}/nodes/{node}` | room mutation |
| `POST /v1/mindmaps/{id}/nodes/{node}/promote` | room mutation + the existing epic/initiative creation |

New:

| route | |
|---|---|
| `POST /v1/mindmaps/{id}/session` | mint a `tkd_` sync credential |
| `POST /v1/mindmaps/{id}/relationships` | create |
| `DELETE /v1/mindmaps/{id}/relationships/{rel}` | remove |

`promote` keeps its current semantics exactly: only direct children become
tickets, the node stays and carries `promoted_kind`/`promoted_id`, and promoting
the same branch twice is refused. Nothing about promotion moves a node.

MCP tools (`takomo_mindmap_new|grow|show|list|promote`) keep their names,
arguments and output shape. `takomo_mindmap_show` still renders the indented text
with ids alongside, because that is the cheapest shape for a model to read.

---

## 8. Events

The current set stands and fires from the API paths: `mindmap_created`,
`mindmap_grown`, `mindmap_moved`, `mindmap_pruned`, `mindmap_promoted`,
`mindmap_updated`, `mindmap_deleted`.

Socket edits emit nothing, which is the rule documents already follow and which
the current mindmap doc already states for text and placement: an event per
keystroke-batch would bury every other event in the project under one person's
typing.

**No new event is added for relationships.** A relationship created in the browser
would emit nothing while one created over the API would emit something, and an
event that fires half the time is worse than no event at all.

---

## 9. Migrating existing maps

Every existing map is converted once, at migration time: read its
`mindmap_nodes` rows, build the Y.Doc, write it as a single initial update into
`crdt_updates`. A Yjs document's whole state serialises as one ordinary update, so
this is the same shape as a compaction.

Field mapping: `text` → `title`; `position` → an `order` key assigned in the
existing sorted sequence; `x`/`y` carried across; `promoted_kind`/`promoted_id`
carried across; `parent` carried across; `notes` empty; `origin` = `human`;
`kind` = `thought`.

`mindmap_nodes` is **left in place and stops being read**. Dropping it in the same
release removes the only way to check the conversion afterwards. A later release
drops it.

---

## 10. Web

`/mindmaps` keeps `web/src/lib/mindmap-layout.ts` untouched. That module holds
every geometric decision as a pure function — where a node lands, what a drop
means, how zoom tracks the cursor — and it is the reason any of the canvas is
testable at all in a jsdom with no layout engine. The state source behind it
changes from REST to the Y.Doc; the geometry does not.

**Fold state is per-viewer** and lives in browser storage next to pan and zoom.
It is not in the CRDT: collapsing a branch must not collapse it under somebody
else mid-conversation. The prototype already treats it this way, holding fold in
client state beside the viewport.

Awareness carries who is on which node, so a collaborator's selection is visible.

### The bundle consequence

`yjs`, `y-websocket`, `y-protocols` and `lib0` currently sit in the
**editor-only** chunk, code-split behind `/documents` and enforced by
`EDITOR_ONLY_PACKAGES` in `web/vite.config.ts`. The first-load budget depends on
that split.

A collaborative mindmap is a second consumer, so the chunk splits in two:

- **collab** — `yjs`, `y-websocket`, `y-protocols`, `lib0`, shared by both routes
- **editor** — `@tiptap`, `prosemirror-*` and their dependencies, still
  `/documents` only

`/mindmaps` becomes code-split the same way `/documents` is. Without this, Yjs
lands in the eager bundle and `npm run size` fails — which is the gate doing its
job, not an obstacle.

Any new component under `web/src/components/` must be exported from
`web/src/components/index.ts` or it is invisible to the design system.

---

## 11. What must be tested

Beyond the standing rule that every new or changed route ships with an
integration test in `tests/api.rs` and an `spec/openapi.yaml` update:

- **the migration** — a database with existing maps converts, every node's text,
  parent, order, placement and promotion link survives, and the map reads back
  identically through `GET /v1/mindmaps/{id}`
- **the contract** — the response shape for an unmodified map is unchanged from
  before the migration, field for field
- **fractional index vectors** — the shared file, checked by both the Rust and
  the TypeScript suite
- **normalisation** — a constructed cycle resolves to the same tree from either
  peer's state; an orphan re-parents to root rather than vanishing
- **concurrency** — two rooms applying interleaved updates converge
- **caps** — each cap refuses on the API path with its teaching message
- **live write** — a node added through MCP appears in a connected socket peer
  without a reload
- **the credential** — a `tkd_` session minted for a mindmap cannot reach a
  document, and vice versa
- **cascade** — deleting a map removes its updates and sessions

Gates to run before wrapping up, since CI is the only wall: `cargo fmt`,
`cargo clippy --all-targets -- -D warnings`, `cargo test --release`, and in
`web/`: `npm run check`, `npm run lint`, `npm test`, `npm run size`.

Verify on a clean worktree with a private `CARGO_TARGET_DIR`. A shared target dir
makes a concurrent `cargo test` result untrustworthy — the integration-test
binaries are not keyed on the workspace and two sessions overwrite the same file.

---

## 12. Deliberately not in this version

- **`gist` on a node** — a one-line summary distinct from both title and notes.
  Wait until the map surface asks for it.
- **Cross-map relationships** — `from`/`to` are node ids within one map. Spanning
  maps needs a qualified reference and a story for what happens when the other
  map is deleted.
- **A trust lens** — `origin` and `reviewed` are stored so the lens is buildable,
  but nothing renders them yet.
- **Map ⇄ document round-trip** — the next conversation, and it needs the
  document's section tree to exist first.
- **Dictation, Cmd-K, question threads** — surface features on top of this model,
  not part of putting it in place.
