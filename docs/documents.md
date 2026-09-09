# Documents: prose humans and agents write at the same time

> **The browser surface this page describes is gone.** `/documents` now renders
> the project specification workspace (see [Specification](specification.md)),
> whose Document view renders the mindmap's own nodes as prose. The standalone document
> editor, its list and its ⌘K menu were removed with it. What survives, and what
> the rest of this page is still accurate about, is the model underneath: the
> `documents` table, `/v1/documents/*`, the `tkd_` sync socket, the Yjs
> update log and the four `takomo_document_*` tools. Read it as the API's
> documentation, not the page's. See `spec/one-model-two-views.md`.


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
| `crdt_updates` | the CRDT **update log**: opaque Yjs blobs, replayed in `seq` order. Named `doc_updates` until it was widened to carry mindmaps too |
| `crdt_sessions` | short-lived tickets for the sync socket, for any collaborative object |

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


## Tables

Use `/table` in an empty paragraph to insert a table and choose its row and column count.
There is no standalone insertion button. While editing a table, one **Table actions** menu
groups row and column changes, headers, merging/splitting cells and deletion. Actions and
selection hints stay inside the menu; no table controls appear outside the active table.
Keyboard and pointer actions preserve the cell selection. Escape returns focus to the table;
clicking elsewhere dismisses the menu without taking focus back. Drag across cells to select a
rectangle for merging; drag a column border to resize. Tab moves between cells. Cells accept
the editor's rich content, including paragraphs, lists, links, and formatted text. Read-only
viewers see tables but cannot change them. These edits use the same Yjs document and undo
history as prose; no separate table persistence exists.

A table is one top-level block with a stable `blk_…` id. Agent reads serialize tables as HTML
inside the annotated Markdown, retaining header cells, `rowspan`, `colspan`, `colwidth`, and
rich cell content. A proposal's `markdown` can contain this HTML table, including blank lines,
or a rectangular Markdown pipe table with a header separator (`---`). Use HTML for merged
cells and rich content; pipe cell text is plain text. Replacing a table remains a whole-block
proposal and requires acceptance, just like replacing a paragraph. Cell-level proposal diffs
and spreadsheet formulas are not provided.

## Insert blocks while writing

In an empty paragraph in a section, type `/` to open the block menu and keep typing to
filter it. Arrow keys select a result; Enter inserts it; Escape closes the menu while
leaving the typed text intact. Lists, quotes, code, tables and diagrams use the
editor's existing blocks. The table picker accepts 1–10 rows and columns; Back or Escape
returns to the command menu. Mermaid, PlantUML, D2 and Wireframe insertions start with
editable source. Their shared Code / View controls render through the configured private
Kroki service; see [diagrams and wireframes](diagrams.md) for setup, limits and offline behavior.

`/h1`, `/h2` and `/h3` ask for a section title and create a real shared section at the
current section boundary. H1 creates a top-level section; H2 and H3 nest under the appropriate
preceding parent. Levels that would skip a parent remain visible with an explanation.
Creation focuses the new section body and leaves surrounding prose in the original section.
Back or Escape keeps the slash query; a refused creation retains the title for retry.
Ordinary slashes in prose or code keep their normal meaning. The existing `#`/`##` and
Ctrl/Cmd + Alt + 1/2/3 formatting shortcuts remain unchanged, including converting a final
heading into a section on Enter.

The menu state is local to the editor. Typed `/query` remains document text until a choice
replaces it, and insertion uses the current section's normal collaboration and undo path.
If the trigger is removed or the caret leaves it, the menu closes. A stale choice cannot
replace text changed by another collaborator. Read-only viewers have no insertion menu.

## Section controls, focus mode and history

Section actions sit behind the ellipsis beside each heading. The menu contains the accessible
colored trust indicator and review/history actions; pending proposals remain visible. The copy
link icon sits directly beside the heading, visible on hover or keyboard focus and always on
touch screens. Clicking prose sets the current section without navigating or scrolling.

The outline is a resizable sidebar when the document pane is at least 850px wide, and a drawer
in a narrower pane. Its toolbar toggle remembers whether it is open. Icon controls fold and
unfold branches. Arrow keys move focus or fold branches; Enter/Space activates a section.
Dragging a row previews a before/after/inside destination. The keyboard/context-menu Move
dialog provides the same operation without dragging. Subtrees keep their prose, links and
identities; cycles and missing destinations are refused.

The document toolbar has one chronological Undo/Redo pair for local prose, formatting,
heading changes, insertions and section moves. History survives section editor virtualization
and the shared session's Doc/Map switch, but not a reload. It is not the audit trace. Yjs selective
undo preserves remote prose; stale structural reversals are refused. Selection uses CRDT
relative anchors that survive editor remounts. New document edits clear the redo branch.

**Focus mode** hides the project rail, outline, history controls and document-wide comments
while keeping the current editor, selection, formatting, save status and view switcher. The
header toggle remains available to exit; the preference is local to the browser. Narrow panes
keep formatting visible and fold secondary tools into **Tools**. **Search document** is the
single document search control, available to readers as well as writers.

H1 and H2 numbering can be hidden independently from the toolbar, with personal preferences
remembered per project and a reset to project defaults. Appearance settings define default
visibility and number size; numbers follow heading typography and color. `/grill` remains
available in the Codex conversation. The outline supports Up/Down/Home/End for focus,
Right/Left for hierarchy navigation, and Shift+F10 or the context menu for Move.

## Formatting, continuous writing, text comments, section references and paste

The compact formatting toolbar follows the current prose selection. Choose Normal text or
Heading 1–3, or apply bold, italic and lists. Mixed selections and unsupported block styles
are named explicitly. Using a control preserves the selected words; changing a paragraph’s
style does not split its section. While editing a section title, prose formatting controls
are unavailable so they cannot change a previous selection elsewhere.

Enter in a title, or ArrowDown at its end, moves into its prose. Enter commits the title as one
line whether or not a modifier is held or part of it is selected, so a title never gains a line
break; only an active input-method composition is left alone. ArrowUp or Backspace at the start
of a top-level paragraph returns to the title without deleting anything. ArrowDown at the
end of a section goes to the next visible title, and ArrowUp at a title’s beginning goes to
the previous visible prose. For that arrow and Backspace navigation, modified shortcuts,
selected text, composition, lists, tables, code and slash-menu navigation keep their existing
behavior. Hidden sections stay folded.

Copy section link in the heading’s actions copies a canonical project/section URL, with
confirmation or a selectable fallback if clipboard access is refused. Stable node identity
keeps these links valid after renaming and moving sections.

The reading layout uses a continuous page with responsive margins and paragraph/list
spacing. Existing global templates and project overrides supply the type scale, heading
spacing and line height; there is no additional configuration layer or pagination.

Select prose and choose Add comment. Threads keep the quoted passage, replies and
open/resolved state in the shared document — a top-level `documentComments` map in the same
Y.Doc as `proposals`, keyed by thread id and carrying the section id, so no route or table is
involved (`web/src/lib/document-comments.ts`). Comments in the section actions opens them.
Click a highlighted passage to open its discussion, or use Show text from the thread to
locate it. Readers can see comments but cannot post, reply, resolve or reopen them. Comments
use the same authorized connection and save status as the document and persist after reload.
Edits outside a passage shift its anchor with the text. If the selected words change or
are removed, the quote and discussion remain with a detached-text indication; they are not
matched to another occurrence. This first version supports plain-text comments on a
selection within one section, without mentions or notifications.

All comments opens a document-wide panel with Open (default), Resolved and All filters, each
carrying its count. A document with no comments at all tells a writer how to add one; a
filter that matches nothing says so instead. Threads
show their section title and support the same replies and status changes as section comments.
Go to text unfolds and selects the section, mounts its editor if needed, and selects the exact
anchored passage. Changed text produces a notice; removed sections retain their discussion
without a misleading navigation link. Readers can filter and navigate without editing.

Type `@` in normal prose to search section titles or numbers. Use Up/Down to choose, Enter to
insert, and Escape to keep the typed text. This works mid-paragraph, in lists and in table cells;
email addresses and code keep a literal `@`. The picker shows up to 50 matches and reports when
more are available; narrow the query to find them. Inserting a chip is one undoable action.

Insert section reference also opens a **Link to section** picker that searches titles and numbers,
reports its result count and shows document numbers to distinguish duplicate titles. It
inserts at the current prose selection. References store the stable section
id and a nested fallback title. Compact reference chips show the current number and title,
following local or remote renames/reorders without writing new prose. Their numbers remain
visible when heading numbers are hidden. Chips have hover and keyboard-focus feedback. Deleted targets show
the fallback with a missing-section label. API and Map text projections resolve the current
target title on every read too; empty titles use “Untitled section”. These reads never modify
the stored fallback or prose. Table exports render references as escaped inline spans, preserving
the surrounding cell text. Copying a reference within the project preserves its
identity and raw fallback across repeated copies, including after renames or language changes;
pasting it into another project retains an ordinary link to the original project.

Pasting HTML automatically removes source fonts, colors, sizes, spacing and copied block ids.
Supported headings, nested lists, tables, code, links and emphasis remain editable and use the
project template. Style-only bold/italic/underline/strike emphasis is retained, including Google
Docs exports. Plain-text paste remains supported; multi-block paste undoes in one step and
redo restores the same block identities. Saved content is not rewritten.

## Discuss a document with Codex

The document's Codex conversation is shared with its project and persists across
refreshes and reopening the document. Use **Ask Codex** from the document commands
or toolbar. Select one or several sections in its context picker, or explicitly
choose **Whole document**. The context is shown before sending and recorded with
each turn; changing the selection does not start a new conversation.

Actions can discuss the context, challenge assumptions, draft test descriptions,
or draft clarification questions. Replies are reviewable Markdown: they do not
create verification checks, open workflow questions, edit the specification or
execute tests. Existing section-level conversations remain separate and readable.
Readers can inspect the shared history; sending requires human and write scopes
and a writable project.

Takomo reads the current shared document on each send, preserving section order,
parent IDs and structured prose (including table boundaries and code languages).
The combined context is capped at 100 KB; oversized requests fail without silent
truncation, so choose fewer sections. Sent context retains its original titles
and text even after later edits. One turn can be queued/running at a time, with
at most 100 turns per document conversation; reaching the limit leaves history
readable and disables new messages. This MVP has no reset or continuation thread.

The Codex service resumes the existing App Server thread and remains bound to the
same worker service ID. Keep that worker's state directory when restarting it.
Losing its Codex thread state is not repaired by browser refresh; failures remain
visible with the stored conversation history. Takomo does not attach arbitrary
terminal Codex sessions through this feature.

Deploy the updated Takomo API before updating the standalone worker. The worker
must advertise `document_chat` in `supported_kinds` when claiming jobs. Older
workers continue consuming their existing job kinds and leave document turns
queued until a compatible worker is available. A queued turn shows when it was
requested, a Refresh status action and a link to the agent queue page; the
document does not report worker availability, and says so rather than
inviting a resend. The worker remains a separate
service; bundling diagram renderers does not install a Codex worker.

### Document workspace and sources

The document chat offers automatic context, explicit selected sections (including
quoted text), and whole-document review. Pins are shared conversation settings:
they survive reloads before a message is sent. Each request captures the current
pins, selection and quote. Selected mode limits every tool to selected sections
plus pins; automatic and whole-document modes can read the full document.
Quotes are checked against captured plain text. If the selection changed, select
it again. Deleted pinned sections can be removed without losing conversation
history.

New requests queue `document_workspace` jobs containing an immutable document
snapshot, section hierarchy, rich prose and deterministic section versions. The
snapshot supports at most 500 total sections and has an 8 MB UTF-8 limit; oversized documents fail clearly without silently
omitting material. This bound applies even when selected context is small, because
the immutable capture stores the complete document. Worker-local outline, lexical
search and bounded section reads use only this snapshot, without live API access
or embedding services. Search ranks titles, section text and hierarchy; it is not
a guarantee that every relevant section was found. Whole-document review tracks
systematic reading coverage and exposes any unread sections rather than claiming
a complete review.

Each completed turn records the sections actually delivered to Codex, their
versions and fully read coverage. History labels come from the captured snapshot,
so later renames do not rewrite prior evidence. Citation links navigate to the
corresponding section; the displayed source version describes the captured turn,
not a claim that the live section remains unchanged. Full snapshots remain in the
authenticated queue inspector for auditing.

Deploy the backend first, then update the agent service: older servers reject the
new `document_workspace` capability and older workers do not claim workspace jobs.
Legacy `document_chat` requests and history remain supported. A preexisting Codex
thread without document tools is migrated once to a new tool-enabled thread on
the same worker, with retained/omitted history counts recorded visibly. Subsequent
turns resume that thread. Preserve the worker service identity and Codex state.
This feature does not install or restart a production worker automatically.

## Ticket references to the project document

In the current specification workspace, a ticket can reference several document
sections. A reference records its captured source version and provenance:
**Original source** for work created from a section, **Manually linked** for a
member's selection, and **Automatically matched** for classifier output. An
optional primary section controls board grouping. This does not change the
ticket's epic, parent, dependencies, state or workflow rules.

Open a ticket's **Document references** panel to inspect accepted links and
suggestions separately. Suggestions include their reason and a source quote.
Accept a suggestion, choose another section, or dismiss it. Choosing another
section adds the manual reference before dismissing that suggestion. Manual
selection supports multiple sections and an optional primary marker; clearing
the primary marker keeps the link. Removed references, captured titles, source
versions and reviewer metadata remain available in the history. A changed or
removed source is marked explicitly; stale suggestions must be classified again
or linked manually to current source before acceptance.

New tickets and material ticket changes are queued for asynchronous document
classification. The agent service must advertise `ticket_document_classify`;
older services cannot claim that work. The classifier uses the same immutable,
project-scoped document retrieval tools as document discussion and returns up to
three candidate sections with verified source versions, quotes and reasons.
Classification is bounded to 500 sections and 8,000,000 UTF-8 bytes. Documents
above that limit report classification as unavailable; existing references and
manual section linking remain usable.
A no-match result is explicit and may indicate a documentation gap. It does not
create a document section or move a ticket. **Find matches again** retries one
ticket; the agent queue inspector exposes status and opens the related ticket.
The classifier never automatically replaces existing accepted references.

Project settings default to **Suggest matches for review**. An administrator can
choose **Automatically accept unique title matches; suggest other matches**.
Automatic acceptance is intentionally narrow: there must be exactly one
candidate with no ambiguity, its title must uniquely equal the ticket title
after whitespace and case normalization (not a broad semantic match), its quote
and retrieval evidence must be verified, both ticket and document versions must
still match, and the ticket must have no accepted references. Other matches
remain suggestions. A project member with human and write scopes can explicitly
schedule existing open tickets using **Find matches for tickets without
references**. Merely opening the board or deploying the feature does not run a
bulk backfill. No document-link workflow gate is introduced.

The board's **Without a document reference** filter includes tickets whose only
references point to removed sections or a previous project. **Group by document
heading** uses the valid primary reference; a ticket without one appears in the
no-primary group. Secondary memberships appear on the ticket card as `+N`,
without duplicating the card into multiple groups. A section's **Associated
tickets** disclosure provides the reverse lookup, with **Load more** and an
explicit shown/total count for larger result sets.

The HTTP surface is:

- `GET/POST /v1/tickets/{id}/document-links`: inspect references or add a manual
  related-section reference (`section_id`, optional `primary` and `reason`).
- `PATCH /v1/tickets/{id}/document-links/{link}`: accept/remove or change the
  primary marker; `DELETE` retains the removed reference in history.
- `POST /v1/tickets/{id}/document-classification`: retry using a `request_id`.
- `GET/PUT /v1/projects/{id}/document-classification-config`: read or set `mode`
  (`suggest` or `auto_apply_clear`); writes require admin scope.
- `POST /v1/projects/{id}/document-classification`: explicitly schedule pending
  work for existing open tickets, with `request_id`.
- `GET /v1/projects/{id}/document-links`: page accepted reverse references with
  optional `section_id`, `limit` (1–500), and `offset` (0–100000).

Read operations require read scope and project access. Reference decisions and
classification requests require human and write scopes in a writable project.
Ticket list/detail projections include `document_refs`; `document_section` and
`document_linked` list filters operate on valid accepted references. Missing
historical references remain inspectable but do not count as valid membership.

### Operator rollout

Deploy the compatible Takomo server first, then upgrade and restart the agent
service with the new ticket-document classifier module and
`ticket_document_classify` capability. Updating the server does not update an
already installed production worker. Older workers continue their supported jobs
but cannot claim classification requests; those requests wait for a compatible
worker. Verify the new worker claims a disposable classification request in the
queue inspector before relying on automatic matching. No existing-ticket bulk
backfill is scheduled by installation; use the explicit project action when
needed.

## Reset a document

In **Settings → Projects → Open → Reset a document**, an administrator can select
that project's specification or an initiative document and clear its current
contents. The first confirmation names the document and explains what will be
removed; the second requires typing its exact ID before **Clear document** becomes
available. Canceling either step makes no changes. Archived projects must be
restored first.

`POST /v1/mindmaps/{id}/reset` with `{"confirm_id":"<mindmap id>"}` clears the
specification summary and all sections, prose, node attachments, relationships,
comments and agent proposals. **The document and mindmap share this content, so
both views are cleared.** The map keeps its identity, title, status and metadata;
existing revision and review history and linked tickets/checks remain. This reset
cannot be reversed with Undo. Copy anything you need before confirming.

The server requires `admin` and access to the project, merges the live replica
with every persisted update it has not replayed, verifies inside the reset
transaction that nothing was appended meanwhile (retrying from a fresh read when
it was), commits before replying, and broadcasts the deletion to connected
editors. The plan's trace (`GET /v1/mindmaps/{id}/trace`) gains one `pruned` act
with no section, noting the reset; every earlier act stays. Existing synced text cannot be revived by
replaying an old replica, and an open editor drops its structural (outline) undo
history, so the cleared sections cannot come back through Undo. New edits,
including previously unsynced concurrent changes, still merge normally. Reset is
not a storage purge or a barrier against new edits. It emits `mindmap_reset`.

The same settings control supports initiative documents:
`POST /v1/initiatives/{id}/reset` with the exact `confirm_id` clears their summary
and entries (including views, notes, amendments, discussions, proposals and
attachments), preserving identity, filing metadata and linked work. It emits
`initiative_reset`.

Legacy collaborative-document API clients can use
`POST /v1/documents/{id}/reset` with the same confirmation shape. It clears prose
and proposals while preserving document filing and metadata, emits `document.reset`,
and refuses archived documents. The current specification UI uses the mindmap
endpoint above.
