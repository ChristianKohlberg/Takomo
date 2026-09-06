# Project specification workspace

The shared header also opens [version history](specification-history.md):
automatic saved CRDT versions, named agreements, comparisons, and downloads.
Reviewing an earlier version keeps the live editor and collaboration session open.

Open `/projects/{project}/specification?view=document`. Document, Map and Tests
are views within this workspace; the project and selected section stay in the
URL. Use `view=map` or `view=tests`, and `section={node-id}` to share a selection.
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
visible views.

The project picker and Inbox sit at the top of the shared navigation rail, not
in the page header; the Inbox icon carries the count of open questions, or a
green check when there are none. The language switch is in the profile menu.

Test definitions keep the readable example separate from its technical details.
Cases with `steps` and `expected` in their assignment show a numbered procedure
and expected result; other parameters remain available in a collapsed section.
Existing `metadata.specification.bindings` on a check appear under **Code
references**: an array of `{file, selector, proves?, limits?}` entries, with an
optional `bindings_source_commit` beside it naming the commit they were recorded
against. An entry missing `file` or `selector` is skipped and the panel says so.
References describe a mapping, not a test result; execution evidence remains in
Runs.
