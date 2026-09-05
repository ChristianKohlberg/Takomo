# Project specification workspace

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

The breadcrumb follows the shared section selection. Section test counts and
failures open a side panel without leaving the document or map; the Tests view
provides the full catalog and can clear the section filter. Check editors keep
their own CRDT sessions. A shared project notification socket refreshes server-owned
metadata and verdicts for the workspace and its visible views.

The project picker and Inbox controls remain in the common page header.

Test definitions keep the readable example separate from its technical details.
Cases with `steps` and `expected` in their assignment show a numbered procedure
and expected result; other parameters remain available in a collapsed section.
Existing `metadata.specification.bindings` appear under **Code references** with
the file, selector, intended coverage and limitations. References describe a
mapping, not a test result; execution evidence remains in Runs.
