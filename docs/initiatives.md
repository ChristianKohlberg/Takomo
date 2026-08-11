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

`/initiatives` is the page. A list on the left with each collection's rollup, one initiative open on
the right: its counts, its entries newest-first with markdown rendered by the same renderer `/board`
and `/inbox` use, and a composer at the top. Title and summary are edited in place; status is a
dropdown; a file picker attaches a document. It is the only SPA in this repo that **writes**.

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
call an MCP tool:

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
| `thread` | `{ pane, para, state? }` | a margin note anchored to a paragraph. `state` is `open` (default), `running` or `resolved` |

Everything else — `transcript`, `sample-data`, `code-research`, `research`, `note` — is **evidence**:
citable from a pane, and listed in the lineage footer.

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
the plain entry log when no pane has been written, and the log is always reachable as a fourth tab.

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
