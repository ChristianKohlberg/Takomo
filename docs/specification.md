# Project specification workspace

The shared header also opens [version history](specification-history.md):
automatic saved CRDT versions, named agreements, comparisons, and downloads.
Reviewing an earlier version keeps the live editor and collaboration session open.
Collaborators appear in the header as a count that opens a list of names with
their session counts; a name identifies a session, not a verified person or role.

Open `/projects/{project}/specification?view=document`. Document, Map and Tests
are views within this workspace; the project and selected section stay in the
URL. Use `view=map` or `view=tests`, and `section={node-id}` to share a selection.
The navigation rail lists the three views under **Specification**, and each link
carries the current project and selected section, so switching views there keeps
the place you were reading.
Older `/documents`, `/mindmaps` and `/verification` bookmarks redirect here,
including section/check selections. A legacy map ID resolves its actual project.

Each project has one plan. A map node and a document section are the same object,
including their title and prose. The workspace owns one plan Y.Doc, sync provider,
durable local update log, save indicator and presence state. Switching views keeps
that connection alive. Visited views retain their local state while React Activity
pauses hidden effects. Project changes dispose the old workspace and its replica.

Opening the workspace with write access provides the project's plan when there
is none yet; there is no create step, and a read-only visit never creates one.
New sections are added inline in Document view and appear in Map at once. Both
rules are specified in `spec/one-model-two-views.md`.

Section test counts and failures open a side panel without leaving the document
or map; the Tests view provides the full catalog and can clear the section
filter. Check editors keep their own CRDT sessions. A shared project notification
socket refreshes server-owned metadata and verdicts for the workspace and its
visible views. Empty worker claims and maintenance sweeps do not send refreshes:
notifications require a committed row change, including changes made by SQLite
triggers. Failed or rolled-back writes do not notify readers. The current
notification channel still coalesces real writes across projects; it is not a
project-filtered event stream. Status polling for active agent work and embedding
progress has its own cadence and remains enabled.

Opening or refreshing an already-migrated document does not append a CRDT update,
even with write access. Server-side document operations persist only newly emitted
transaction updates, including deletion-only edits. Encoded state diffs can contain
framing bytes and old deletions without a new change; these are not treated as
writes. This prevents a document read from starting a flush/refresh/read loop.

The project picker sits at the top of the shared navigation rail, not in the
page header — on a phone, in the compact top bar that stands in for the rail.
Inbox is the rail's footer entry, directly above the profile block; it carries
the count of open questions, or a green check when there are none. The language
switch is in the profile menu.

The Definitions tab summarizes how many definitions are not run, verified,
failed or outdated, narrows by that status, by text and by section, and links
each card to its source section. An empty Runs tab explains that runs are
created from definitions and offers a way there; a section filter that matches
no run says so instead. A permission failure is shown once, naming the project,
in place of an empty list.

Test definitions keep the readable example separate from its technical details.
Cases with `steps` and `expected` in their assignment show a numbered procedure
and expected result; other parameters remain in a collapsed section as labelled
values with a copy action, the raw JSON beneath them.
Existing `metadata.specification.bindings` on a check appear under **Code
references**: an array of `{file, selector, proves?, limits?}` entries, with an
optional `bindings_source_commit` beside it naming the commit they were recorded
against. An entry missing `file` or `selector` is skipped and the panel says so.
References describe a mapping, not a test result; execution evidence remains in
Runs.
