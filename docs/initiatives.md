# Initiatives: a home for an idea before it is work

A ticket is a unit of work. It has a workflow, it gets claimed, it closes. An initiative is the
other thing a fleet produces: a product idea, a direction, what is left over after a good
conversation with an agent. It is **nurtured**, not completed — a colleague is asked for feedback, a
few agents are told to go research, and each of them adds to it over time.

There is deliberately no workflow here, no claim, no lease, no fence and no ready queue. Nothing
races for an initiative, because appending is not exclusive: two agents adding findings at the same
time both belong, and the append-only entry log keeps both.

## The shape

An initiative carries a **quick title**, a **very short description**, a status label, labels, tags
and free-form metadata. Everything else lives in its **entries** — one per contribution.

An entry is generic in two directions:

- `kind` is a free-form slug (`note`, `research`, `feedback`, `transcript`, `document`,
  `decision`, …). A new sort of input needs no schema change.
- It may carry markdown `text`, an attached document, or both.

**Provenance is first-class**, not left to a free-form note. Every entry says where it came from:

| field | what it records |
|---|---|
| `source` | **required** — an agent id, a person, a conversation (`agent:w1`, `person:ada`, `claude:chat`) |
| `source_uri` | where it lives: the conversation, the doc, the PR |
| `origin_at` | when the content was **written** |
| `created_at` | when it **landed here** |

The last two are separate on purpose. A transcript pasted in a week later has two different, both
correct, timestamps.

## The rollup — what a UI shows at a glance

Every read of an initiative carries a derived `rollup`, so a reader can see how much has piled up
without reading any of it:

```json
"rollup": {
  "entries": 7, "attachments": 2, "chars": 18450,
  "bytes": 412_003, "attachment_bytes": 393_553,
  "megabytes": 0.39, "last_entry_at": "2026-07-30T14:02:11.980Z"
}
```

It is **never stored**. It is recomputed from the entries on every read, which is what keeps it from
drifting from the thing it describes. `chars` counts characters of entry text — the number a human
means by "how long is this" — while `bytes` is what is actually stored, text and attachments
together.

## Three surfaces

`/initiatives` is the page: **an explorer, a document, and what you can do with a passage**. A folder
tree on the left over every document in the project; one document rendered in the middle as a single
scrolling surface; a pane on the right that turns a highlight into an action. Title, summary and
folder are edited in place; status is a dropdown; a file picker attaches a document. It is the only
SPA in this repo that **writes**.

**Folders are `metadata.path`, and nothing else.** Initiatives stay flat in the store — `metadata`
was already a free-form JSON object on every one of them, so nesting needed no migration, no folder
table and no orphaned-directory problem. A folder exists because a document names it, which means the
last document leaving takes the folder with it. Moving one is a `metadata_merge` of its path.
`web/src/lib/initiative-tree.ts` derives the tree on every read, the same never-stored rule the
`rollup` and the document follow.

Initiatives are usually created and fed over **MCP**, because the thing that produces one is an
agent in a conversation, not a form.

| tool | |
|---|---|
| `takomo_initiative_new` | open one: project, title, summary, labels, tags |
| `takomo_initiative_append` | add one entry — the verb that matters |
| `takomo_initiative_update` | edit the description: title, summary, status, labels, tags, metadata |
| `takomo_initiative_list` | *(read)* filter by project, status, label, tag, text |
| `takomo_initiative_show` | *(read)* one initiative, its rollup, and a page of entries |

REST is what the page drives — reads, plus the three writes the page needs, since a browser cannot
call an MCP tool. Note what is **not** here: the overhaul added no route. Folders are `metadata_merge`
on the existing PATCH, every document action is an append through the existing entry route, and
filing a passage as work or a question uses `POST /v1/tickets` and `POST /v1/questions`, which
already existed.

```
GET   /v1/initiatives?project=&status=&q=&label=&tag=&limit=&cursor=
POST  /v1/initiatives                                   # create
GET   /v1/initiatives/{id}
PATCH /v1/initiatives/{id}                              # title, summary, status, labels, tags, metadata
GET   /v1/initiatives/{id}/entries?limit=&cursor=
POST  /v1/initiatives/{id}/entries                      # append
GET   /v1/initiatives/{id}/entries/{entry}/content
```

Both surfaces go through the same `Store` methods and share `decode_attachment` and
`parse_rfc3339_ms`, so neither can drift into accepting what the other refuses.

Note what the page cannot do: **edit or delete an entry**. Entries are append-only, and no route
exposes otherwise. The accumulated record is the point.

One browser detail worth knowing if you touch the page: the attachment route needs the bearer token,
so a plain `<a href>` cannot fetch it — the browser would send an unauthenticated request and get a
401. The page fetches with the header and hands the blob to a throwaway object URL.

Entry lists carry text but never attachment bytes — `has_content` says whether there are any, and
the content route serves them. That route is the only endpoint in the API returning something other
than JSON, and it is why `content` is never selected by any other query: a document is fetched by
itself, once, by the reader that wants it.

## The document: three views reduced from the same entries

Read newest-first, an entry log tells you what was **worked on**. It does not tell you what is now
**understood** — and an initiative fed for months is exactly where those two come apart. So two
reserved entry kinds turn the same rows into a document, without a schema change: `kind` has always
been a free-form slug and `meta` a free-form JSON object on every entry.

| kind | `meta` | what it produces |
|---|---|---|
| `view` | `{ pane, cites: [entryId, …] }` | one pane's prose. `pane` is `business`, `technical` or `verification` |
| `view` | `+ proposed: true` | an **amendment** — offered as a diff, never live until someone accepts |
| `view` | `+ proposed: true` + an anchor | a **suggestion** scoped to one highlighted range |
| `thread` | `{ pane, state?, ticket?, supersedes? }` + an anchor | a note on a passage. `state` is `open` (default), `running` or `resolved` |
| `decision` | `{ accepts \| rejects: <proposalId> }` | a human's verdict on an amendment |
| *(any)* | `origin: true` | the words the idea **arrived** in — quoted above every pane |

## Anchors: how a note stays attached to its words

An **anchor** is `{ quote, prefix, suffix, para }` on the entry's `meta` — the highlighted words plus
32 characters of context on each side. Resolution searches the *current* prose for them and reports
how it found them:

| | |
|---|---|
| **exact** | prefix + quote + suffix still adjacent — certainly the same passage |
| **moved** | the quote survived but its surroundings changed; it may have drifted in meaning |
| **orphaned** | the words are gone. The note is listed under the prose, struck through, never hidden |

Four passes, weakest last, first hit wins: full context, then one side of it, then the bare quote
**only where it occurs once**, then whitespace-insensitively for prose that was merely reflowed. An
ambiguous quote with no surviving context orphans rather than highlighting a coin flip — a confident
highlight on the wrong sentence is worse than admitting the note came loose.

This replaces anchoring to a paragraph *index*, which was clamped into range on read: a note whose
paragraph disappeared silently slid onto a paragraph it was never about, and nothing could tell that
it had. Entries written before anchors existed still carry only `para` and still work — they are
paragraph notes, and `anchor` is null on them.

`web/src/lib/initiative-anchor.ts` is the whole of it, pure and offset-based, with one DOM helper
that converts a browser selection into plain-text offsets. A citation mark displays a bare number
while the prose says `[3]`, so that helper substitutes each mark's source form — which is what keeps
selection coordinates in the same space as the prose, the anchors and the diff.

Everything else — `transcript`, `sample-data`, `code-research`, `research`, `note` — is **evidence**:
citable from a pane, and listed in the lineage footer.

## Every action is an append

Nothing in the document is ever edited or deleted, which is what makes the argument that produced
the current text still readable:

| doing this | appends |
|---|---|
| revising a pane | a new `view` for that pane; the old one stays |
| commenting on a passage | a `thread` carrying the anchor |
| suggesting different words | a proposed `view` carrying the anchor and the replacement |
| acting on a note | a ticket, plus a `thread` carrying `supersedes` and the ticket id |
| asking a person | a ticket, an **advisory question** on it, plus that same `thread` |
| attaching a source | a new `view` with the mark spliced in after the words it supports |
| accepting a suggestion | the amended prose as a real `view`, **plus** a `decision` naming it |
| rejecting one | a `decision` alone — the live prose is untouched |

Accepting a **range-scoped** suggestion is still a plain append of complete prose: the replacement is
spliced into the live pane, the paragraphs are re-serialized with citation marks renumbered from
scratch, and the result is appended as an ordinary `view`. Renumbering matters — a mark caught inside
a replaced range is dropped, because the words it supported are the words being replaced, and a hole
in the numbering would make every later mark point one source too far.

Several suggestions can be pending at once, because two readers highlighting two different sentences
have not collided. A pane-scoped proposal remains single: it is a take-it-or-leave-it rewrite of the
whole argument, and stacking those was never useful.

**Asking a person is a question on a ticket**, not a fifth mechanism. A question hangs off a ticket by
design — a decision nobody can route to a piece of work is a decision that never comes back — so the
passage is filed as a ticket and the question routed against it `advisory`, recording the decision
without parking work nobody has claimed.

Accepting is deliberately two entries rather than one. The `view` is what makes the wording live;
the `decision` records who agreed and keeps the proposal from being offered a second time. A
`decision` on its own changes no prose, which is exactly what a rejection should do.

**Dispatch is a ticket, not a new mechanism.** A margin note becomes work by being filed into the
project's ready queue — the one the fleet already pulls from — tagged `initiative:<id>`. There is no
scheduler, no agent-spawning and no new route: the server stores, the fleet computes.

## The part that is not code

The document only exists if agents write it. `takomo_initiative_append`'s description tells them to:
append the finding as evidence, then say what it *means* with a `view`; use `proposed: true` when a
finding changes what a pane already says rather than adding to it; anchor a doubt as a `thread` next
to the sentence that caused it; mark the arriving words with `origin`.

That is a prompt, not an invariant. Nothing enforces it, and an agent that ignores it leaves an
initiative that renders as a plain log — which is the honest failure mode, not a broken page.

**Citations are local to write and global to read.** A `[3]` mark in a pane's prose indexes that
pane's own `cites` array, so an agent writing one pane never has to know what another pane cited.
The reader-facing number is assigned across the whole document, in pane order, so two panes citing
the same entry show the same number. A mark pointing at nothing — an index past the end, or an id
outside the page of entries — is left as **literal text**. A broken citation that silently vanished
would read as an uncited sentence, which is a lie about where the sentence came from.

**A paragraph with no citation is marked.** Not as a failure, as a fact: it is an assertion nobody
sourced, and it should look like one. That is what makes the prose checkable rather than merely
confident.

The document is **reduced from the entries on every read and never stored** — the same rule the
`rollup` follows, for the same reason: a cached summary drifts from the rows it summarises and
nothing notices. Latest `view` per pane wins, ties broken by id so the winner is deterministic. A
pane is revised by appending a new `view`, never by editing one; the log stays append-only and every
earlier revision is still there.

`web/src/lib/initiative-doc.ts` is the whole derivation, and it is pure — `buildDoc(entries)` with
no I/O, which is why it is unit-tested rather than driven through a browser. The page falls back to
the plain entry log when no pane has been written, and the log stays reachable below the document —
the document is a reduction of it, and a reduction you cannot check against its source is a summary
you have to take on faith.

The three panes render as **sections of one scrolling document** rather than as tabs. That is what
makes highlighting a single gesture: a reader drags across a sentence without first deciding which
pane owns it, and the anchor records the pane afterwards from where the selection landed. Tabs made
the pane a mode you had to be in; sections make it a place you scroll to.

One limitation worth knowing: pane prose renders as **plain paragraphs**, not markdown. Citation
marks are parsed into real elements, so the text cannot go through the markdown renderer without
teaching it about marks first. Formatting inside a pane is therefore not available yet.

## Status is a label, not a state machine

`open` is being fed. `parked` is deliberately set aside — still readable, still appendable; parking
is not closing. `distilled` records that its substance became tickets, which is the only outcome an
initiative has, since it is never "done" on its own terms. Nothing enforces an order between them.

## The caps, and why they exist

An initiative is the one thing in this store that accepts binary uploads, and every write goes
through the single `IMMEDIATE` transaction behind the process-wide write mutex — the mutex whose
serialization *is* the exactly-one-claimant guarantee for the ready queue. An unbounded attachment
would stall every claim, transition and heartbeat in the process for as long as it took to land. So:

| bound | value | error |
|---|---|---|
| one attachment | 5 MiB, checked on the **decoded** bytes | `initiative.attachment_too_large` |
| entries per initiative | 1000 | `initiative.too_many_entries` |
| bytes per initiative | 1 GiB | `initiative.too_large` |
| entry text | 128 KiB | `validation.initiative_field_length` / `validation.entry_text` |

Host anything larger elsewhere and reference it with `source_uri`, putting a summary in `text`.

Two smaller rules worth knowing. An attachment needs a `filename` and/or a `mime`, because bytes
nobody can label are bytes nobody can use. And `mime` must be a bare `type/subtype`: it is served
back verbatim as the attachment's `Content-Type`, so it is restricted to what a header may hold.

## Tags are the same registry as tickets

`person:ada` on an initiative means what it means on a ticket, and resolves through the same
project tag registry — an unknown handle is registered on the fly. That is what makes "who gave this
feedback" and "who owns this ticket" the same question with the same answer.
