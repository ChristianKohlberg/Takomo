# One plan, two renderings: the map and the document

Status: **built** · supersedes the conversion in `spec/mindmap-crdt.md` §"Writing it up", which is deleted

> Mindmap and Document is just a different way of rendering the same
> information. Both have their place. Mindmap for fast brainstorming and
> grouping of topics, document view to spec out the details and see better
> diffs.

A project has **one plan**. The map and the document are two ways of looking at
it, not two things kept in step.

---

## 1. Why the conversion has to go

`POST /v1/mindmaps/{id}/documents` **copies** the map into `documents` rows. It
is a one-way transform run by hand, and that is the whole problem: a node's
`notes` and its document's prose are two places one paragraph lives, and they
disagree after the first edit. Rename a node and the document is stale with
nothing saying so. It reads as two features because it is two features.

Linking two copies is not the same as having one thing. The conversion is
deleted, not extended.

## 2. The shape

**One tree. Two renderings.**

- A **node is a section**. Its title is the heading, its prose is the section.
- **Depth is heading level**; H1 down the tree, as the map already knows.
- **Tree order is reading order.** The document is the depth-first walk.
- Both views write the same CRDT, so they cannot drift, and two people can be in
  different views of the same thought at once.

There is **one document per project** — the plan. Not one per node: the sections
are sections of it. A section that outgrows its home can later be split into a
sub-document that leaves a placeholder behind, which is the only reason a second
document ever appears.

### Where the prose lives

Inside the node, as its own `XmlFragment` in the map's Y.Doc.

This is available rather than hoped for: Tiptap's collaboration extension takes
either a top-level `field` **or** a `fragment` handed to it directly —
`this.options.fragment ? this.options.fragment : document.getXmlFragment(field)`
in `@tiptap/extension-collaboration`. A section's editor binds to the node's own
fragment and edits it in place.

What it buys: the canvas gets each section's first line for free, because it is
in the document the canvas already has open; deleting a node deletes its prose;
one socket, one log, one set of peers for both views. `notes` disappears, having
always been a poorer version of this.

**The trade, stated plainly.** Per-section fragments mean the document view is a
column of section editors rather than one continuous text field, so typing does
not flow across a section boundary — pressing Enter at the end of 2.1 does not
begin 2.2. For a view whose job is to spec out details that is an acceptable
price, and it is what buys per-section history in §4. If continuous typing turns
out to matter more, the alternative is one fragment for the whole plan with
structure derived from its headings — and then the tree is derived from prose
rather than the other way round, which gives up the move, repair and grouping
the map is good at. **Recommendation: per-section, and revisit after use.**

## 3. People, not capabilities

The map currently records `created_by` as a free-form actor string and
`origin` as `human | agent`, derived from whether the token carried the `human`
scope. That is a capability standing in for a person, and this codebase already
has the rule against it: identity travels on `AuthCtx.user` and is **never** a
scope string, because scopes are free-form and a `user:…` scope would be a
forgeable identity — `a_user_scope_string_cannot_forge_assignee_identity` is the
guard, and `cases.human_user` / `case_verdicts."user"` are the precedent, where
the actor string is kept only as the free-form thing the credential carried.

So:

- a node records the **person** who created it (`users.id`), not only the actor;
- every trace entry (§4) records the person behind the credential;
- the trust reading becomes "Ada confirmed this" rather than "a human confirmed
  this", and "nobody has looked at this" means nobody, by name.

An agent's token bound to somebody's automation still records that person —
"whose agent" is worth knowing, which is why `case_verdicts` keeps the user for
an agent verdict too.

`origin` stops being a stored flag and becomes a reading: authored by a person,
or by an agent acting for one.

## 4. History is the point, not a later feature

Every section carries a **trace**: an append-only record of what happened to it,
by whom, and when. `authored`, `edited`, `proposed`, `accepted`, `rejected`,
`reviewed`, `renamed`, `moved`, `split`.

The trace is what makes the document view worth having — it is where "better
diffs" actually comes from — and it is also what the trust lens should read
instead of a boolean: a section nobody has reviewed since it last changed is
*moving*, one confirmed after its last edit is *solid*, one written by an agent
and never read is *unsure*.

**It lives in SQL, not in the CRDT.** Three reasons: it references `users(id)`
and wants a real foreign key; "everything Ada reviewed this week" is a query, not
a document walk; and it must survive compaction, which rewrites the update log
by design.

**It is sparse.** Keystrokes reach nothing — the same rule the event log already
follows, and for the same reason: an entry per keystroke-batch buries every
entry worth reading. A trace entry is an act somebody would name.

The CRDT update log stays what it is: the mechanism that rebuilds the text. The
trace is the record of what people did to it. Two different questions.

## 5. Proposals stay exactly as they are

An agent proposes; a person accepts. That rule survives untouched — it is the
best thing in this codebase and the whole reason the document surface exists.

What changes is only the target: `docprops` operates on a fragment and a
proposals map, so it is re-aimed at a node's fragment with its proposals beside
it. Accepting one writes a trace entry naming the accepting person. The agent
tools grow a "which section" argument; their contract otherwise holds.

## 6. What this replaces

| built | becomes |
|---|---|
| `POST /v1/mindmaps/{id}/documents` | gone — nothing to convert |
| `mindmapdoc::plan_documents`, `store::prose` | gone |
| the node's `document` field, the map's `metadata.document` | gone |
| node `notes` (Y.Text) | node `prose` (XmlFragment) |
| node `origin`, `reviewed` | the trace, plus the person who authored it |
| documents made by conversion | migrated into their nodes, then archived |

`/documents` keeps its place for documents that are not the plan, and for the
sub-documents a split will create. It stops offering to make one inside a
project that has a map.

## 7. What was built

All five, in this order, each landing on its own:

1. **Node prose and identity** — `prose` as a nested `XmlFragment` on the node,
   the legacy `notes` moved into it once on first open and then removed,
   `created_by_user` recorded from `AuthCtx.user`.
2. **The trace** — `plan_trace`, the acts that deserve an entry, and `standing`
   on the map read as a derived reading rather than a stored flag.
3. **The document view** — `/documents` is the plan: the rail is the node tree
   numbered by position, the column is the sections with an editor bound to each
   node's own fragment. The conversion is deleted, with no migration: the branch
   never shipped it.
4. **Proposals re-aimed** — `GET /v1/mindmaps/{id}/prose`, `POST …/proposals`,
   and `takomo_plan_read` / `_propose` / `_proposals`, plus `POST …/run` for the
   one route that calls a model. An agent proposes; a person accepts. Unchanged.
5. **History with two sides** — each act that changed the prose keeps what it
   then said, which is what a diff is made of. Affordable only because the trace
   is sparse.

## 8. The order it was done in

1. **Node prose and identity.** `prose` fragment on the node, `notes` migrated
   into it, `created_by_user` recorded, the canvas reading the first line.
   Nothing new is visible; everything keeps working.
2. **The trace.** The table, the writes on the acts that deserve one, and the
   trust reading derived from it rather than from a flag.
3. **The document view**, replacing the conversion: rail from the node tree,
   column of section editors, the trace visible per section. The conversion and
   its scaffolding are deleted here and its documents migrated.
4. **Proposals re-aimed** at sections, and the agent tools' new target.
5. **Diffs**, once there is a trace to hang them on: what changed in a section
   between two points, presented for reading.

1–3 are the overhaul. 4 keeps agents honest. 5 is the payoff.

What survives untouched: the CRDT foundation, the shared log, the fractional
index, the tree repair, relationships, attachments, the outline model, the
canvas. What is thrown away is the conversion — about a day, mine — and that is
the right trade rather than a sunk cost to defend.
