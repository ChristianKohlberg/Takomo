# Documents: prose humans and agents write at the same time

A ticket is work. An initiative is an idea being nurtured. A **document** is the text itself —
edited live, by several people and several agents at once, with nobody overwriting anybody.

It is built **beside `/initiatives`, not over it**. Nothing here writes to an initiative and nothing
there changed. A document may record the initiative it was distilled from, which is what makes an
eventual migration expressible rather than guessed at.

## Why this exists

The initiative document is *reduced* from an append-only entry log: the latest `view` entry per pane
wins. That was a reasonable way to get a document surface out of rows that already existed, but as a
merge strategy it is last-write-wins — revising a paragraph means appending a whole new copy of the
pane, and whatever somebody else wrote in the meantime loses.

The failure this removes is specific. An agent asked to tighten a paragraph spends several seconds
thinking. If it then writes a document back, everything typed during those seconds is gone, and a
one-word fix arrives as a whole-document diff nobody can review.

So the prose is a **Yjs CRDT**. Every participant — browsers and agents alike — is an ordinary peer
holding a replica. Merging is the data structure's problem rather than a policy anybody has to
remember.

## The shape

| | |
|---|---|
| `documents` | the **filing**: title, folder, status, and the initiative it came from |
| `doc_updates` | the CRDT **update log**: opaque Yjs blobs, replayed in `seq` order |
| `doc_sessions` | short-lived tickets for the sync socket |

There is deliberately **no `body` column**, and no JSON route accepts prose. A text column would be
the last-write-wins merge this exists to remove, wearing a different hat. `version` therefore counts
metadata edits only — CRDT updates arrive by the thousand and would make an `If-Match` precondition
meaningless.

`path` is a folder, and a folder exists only because a document names it — the last document to
leave takes the folder with it. Same rule `/initiatives` derives its tree from, so there is no folder
table and no orphaned-directory problem.

## Yjs is in the binary, not beside it

The obvious way to get Yjs is Hocuspocus, which is Node: a second process to deploy and a second
store to keep consistent, against a repo whose shape — one Rust binary over one SQLite file — is what
`render.yaml` and the Dockerfile actually depend on.

[`yrs`](https://github.com/y-crdt/y-crdt) is the official Rust port and is wire-compatible with the
browser library, so a stock `y-websocket` provider connects to `src/api/docsync.rs` unmodified.

The protocol is implemented directly rather than through `yrs-axum`, which pins `yrs ^0.18` against a
current 0.27 — taking it would either freeze the CRDT three years back or put two incompatible `yrs`
versions in one tree. What it would have provided is about a hundred lines.

Awareness (who is where, live carets) is **relayed and never parsed**. The server is not a
participant, so a replica of presence could only be a stale third opinion about a fact the peers
already hold.

## The debounce is load-bearing

Every mutation in this store runs as one `IMMEDIATE` transaction behind a process-wide
`Mutex<Connection>`, and that serialization *is* the exactly-one-claimant guarantee for the ready
queue. Persisting a keystroke would put every claim, transition and heartbeat in the process behind
somebody's typing — the same hazard the initiative attachment caps exist to prevent, arriving
continuously instead of once.

So the split is: **broadcast is memory, persistence is batched.**

- Applying an update and fanning it out to the other peers touches no database at all.
- A room accumulates updates and flushes them, merged into one blob, every two seconds — and once
  more when the last peer leaves.
- A crash therefore costs at most one flush interval of typing. That is the trade, taken
  deliberately, and it is why the interval is small.

A room exists only while somebody is editing: hydrated from the log on the first join, dropped after
the last leave has flushed. Nothing is cached between sessions, so it cannot become a second,
divergent copy of the store.

**Compaction needs no second table.** A Yjs document's whole state serializes as a single, ordinary
update, so compacting is `DELETE` the rows and `INSERT` the merged blob — same format, same table.

## The fifth credential, and why there is one

A browser `WebSocket` cannot set an `Authorization` header. That is the same limitation that already
keeps `/board` and `/inbox` polling `GET /v1/events` instead of using the SSE stream — but polling is
not an option for a CRDT, so the credential has to ride the handshake.

Putting a real `tk_` token in a query string would scatter the org's actual credential through every
access log on the path. So `POST /v1/documents/{id}/session` mints a `tkd_` ticket instead, following
the `tks_` share and `tka_` answer-grant precedent in `src/auth.rs`:

- **one document** — checked against the path, not trusted from it
- **expiring** (12 hours: long enough to survive the reconnects a flaky network produces, since
  `y-websocket` retries with the URL it was given)
- **revocable**, and swept once long dead
- **no more than its minter** — `can_write` is copied from the calling token's scopes, so a `read`
  token joins as a read-only peer whose edits are never applied

`GET /v1/docsync/{id}` sits outside every bearer middleware, for the reason `/oauth/*` does: it
authenticates with a credential of its own, and a middleware demanding a different one would make it
unreachable.

The document id is the **last** path segment, which is a wire requirement rather than a preference.
`y-websocket` composes its own address as `serverUrl + "/" + room + "?" + params`, so a route shaped
`/v1/documents/{id}/sync` gets assembled as `…/sync?ticket=X/doc-abc` — a path nothing routes, and
the failure is silent: the editor mounts, the page looks right, and the second peer simply never
sees anything. That is how it shipped in the first draft and how the browser check caught it.

## Block ids

Every top-level block carries a stable `blk_…` id (`web/src/lib/block-id.ts`, ported from the doctest
prototype). This is the hinge the rest of the design hangs on rather than a convenience: an agent
that returns **operations against block ids** never touches a block it did not name, so a human
editing three paragraphs away keeps their words. An agent that returns a document cannot make that
promise however it is prompted.

Ids live in a ProseMirror node attribute, so they are part of the CRDT and merge like everything
else. Two peers splitting the same paragraph concurrently can both mint one; the plugin keeps the
first occurrence and reissues the rest, because a duplicate id is worse than a missing one — an agent
op would then address two places at once.

## The page

`/documents` is the only **code-split** route in the app. Tiptap, ProseMirror and Yjs come to ~164 kB
gzipped, more than the rest of the app put together, and every other surface would pay for them on
first paint. Two things make the split real rather than cosmetic:

- `build.rs` embeds a **generated** asset manifest from `web/dist/assets/`, replacing four
  `include_str!`s named by hand — a dynamic `import()` emits a chunk whose name is not knowable when
  Rust compiles.
- `web/vite.config.ts` keeps the editor's packages out of the shared `vendor` chunk. A blanket
  "everything in `node_modules` goes to vendor" sweeps them back onto the critical path while the
  build output still shows a neat little `Editor.js`. `EDITOR_ONLY_PACKAGES` is exactly the set the
  editor install added to the lockfile, transitive names included — `linkifyjs` and `lib0` do not
  look editor-shaped and are.

`npm run size` derives first load from what `index.html` actually references, so a lazy chunk is not
charged against the budget and a statically-imported one still is.

There is no save button and no dirty state, which is the honest UI for a CRDT: "did my change save"
is replaced by "am I connected", which the status line reports.

## The agent: proposes, never writes

This is the half KONZEPT is actually about — *„Der Agent schlägt vor, der Mensch entscheidet."*

Four MCP tools: `takomo_documents`, `takomo_document_read`, `takomo_document_propose`,
`takomo_document_proposals`.

**An agent returns operations, never a document.** It reads the prose annotated with block ids and
replies with ops against them:

```json
[{"op": "replace",      "id": "blk_7f3a", "markdown": "## Pricing\n…"},
 {"op": "insert_after", "id": "blk_7f3a", "markdown": "…"},
 {"op": "delete",       "id": "blk_9c1e"}]
```

Blocks it does not name are untouched, so somebody editing three paragraphs away keeps their words.
That is a property of the vocabulary, not of the prompt — telling a model to stay in its lane is not
the same as knowing it did.

**Nothing it sends becomes live text.** The proposal is stored in a `proposals` map in the same
Y.Doc, beside the prose, and a person accepts or rejects it. Being in the CRDT is what makes it
appear in an open browser immediately and survive a disconnect; a proposal parked server-side until
someone reloaded would be a second source of truth about the same document.

**The read is of the live replica, not the persisted log.** The log is up to one flush behind, so an
agent reading from it would get block ids people had already moved past — and then every op it wrote
would be dropped as stale. `open_room` puts the agent on the same replica the browsers are on.

**Rust reads; the browser writes.** Turning markdown into ProseMirror nodes means knowing the
editor's exact schema, and the editor is the only thing that does. So `src/api/docprops.rs` walks the
CRDT to read and only ever writes the proposal record; `web/src/lib/doc-ops.ts` does the applying.
The asymmetry is deliberate — Rust writing nodes it half-understands is how a shared document gets
quietly corrupted.

**Scope is enforced, not requested.** A run may name the block ids it may touch; an op outside them
is dropped and reported in `skipped`, which is also shown to the reviewer — a proposal smaller than
the agent intended must not be accepted as if it were whole.

**A decision is recorded, not erased.** Accepting applies the ops *and* marks the proposal
`accepted`; rejecting marks it `rejected` and changes no prose. Both stay in the panel, because "we
considered this and said no" is what you want three weeks later when it is proposed again.

Highlighting the affected blocks uses ProseMirror **decorations**, never marks: a decoration is a
local view artifact that touches no CRDT, so the document really is unchanged while a proposal is
pending. A mark would have been content — synced, merged, undoable — quietly breaking the rule it
was drawn to illustrate.

## The prompt bar: the one place Takomo calls a model

⌘K (or the dashed bar under the document) opens a contextual menu. What it offers depends on what is
selected — a highlighted sentence gets different actions from a heading, and the filter box doubles
as free text, so the menu is never a dead end. Choosing runs immediately, because the result is
already a proposal nobody has accepted; a second "are you sure?" would protect nothing.

`POST /v1/documents/{id}/run` is **the only route in this server that calls a language model**, and a
deliberate exception to the "Takomo stores, the agent computes" division everything else keeps. The
alternative was for a person typing "tighten this paragraph" to file a request that sat until some
fleet agent happened to look at that document, which is not a feature anyone would use.

Three things keep the exception contained:

- **It is off unless configured.** No `TAKOMO_TENSORX_API_KEY`, no bar: the route answers a teaching
  503, `GET /v1/whoami` reports `features.doc_agent: false`, and the page explains the absence rather
  than offering something that fails. A deployment that never sets it is the server documented
  everywhere else.
- **Nothing it produces is trusted.** The answer goes through the same `validate_ops` a fleet agent's
  ops do — ids checked against the live document, scope enforced, unusable ops dropped and reported.
- **Nothing it produces is live text.** It writes a proposal, exactly like MCP does.

Provider is TensorX (OpenAI-compatible), default model `deepseek/deepseek-v4-flash-0731`. Structured
outputs constrain the answer to the op schema, with a fallback to `json_object` plus the schema in
the prompt for models that refuse `json_schema` — validated either way, so the fallback costs
strictness, not safety.

### The anti-fabrication rules are load-bearing

Carried over verbatim from the prototype, which measured the failure: on the same task one model
**invented statistics** that were nowhere in the document. A fabricated number that reads well is the
worst thing a document curator can produce, because it is what a reviewer is least likely to catch.

They reduce it; they do not eliminate it. In a real run against this repo's own sample document, the
model turned a paragraph into a commitment and added "*prüft er anhand der E-Mail-Adresse*" — a
detail the neighbouring paragraph makes plausible but the document never states. That is a good
proposal and a wrong sentence at the same time, and it is exactly why nothing reaches the prose
without somebody pressing Accept.

### A no-op is refused

A `replace` whose markdown is the text the block already has is dropped and reported, and a proposal
where every op was one fails with `validation.document_unchanged`. This is not defensive
programming: a model asked to add an open question answered with the block's existing text verbatim
while its summary described the question it had not written. Stored faithfully that is a change a
reviewer reads, accepts, and gets nothing from — with a summary that told them otherwise.

## What is not here yet

- **Block-level diffs, not word-level.** A replaced block shows whole red/green rather than the
  changed words. The op is written at block granularity anyway, but a word diff would read better.
- **No run history.** The prototype feeds the last ten runs back into the prompt, which is what lets
  a follow-up like "the table looks good, but x is missing" resolve to a block id instead of a guess.
- **Commitments (Zusagen) and the icicle map.** Each needs this stage underneath it.
