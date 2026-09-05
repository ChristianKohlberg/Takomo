# Specification history

Document and Map edit the same CRDT document. **Version history** in the shared
specification header lets readers revisit saved content, compare two versions,
and name an agreed version without disturbing the live editor. The selected
versions are URL parameters, so a link or reload reopens the same comparison.
The history list receives updates through the workspace's existing project socket.

Every mindmap CRDT flush containing new operations now archives its update **in the same SQLite
transaction** as the live update. This includes browser WebSocket edits, REST
mutations and CLI/store edits. Versions cover sections, structure, rich prose,
relationships, attachments and map presentation. They do not version test
definitions, execution results, or SQL project/map metadata such as the map's
administrative title/status. Test runs retain their own immutable definition and
section snapshots; browsing history does not change their evidence.

Empty updates and repeated insertion/deletion ranges from sync retries do not create
versions. Out-of-order operations are retained, including gaps in client clocks.

A version is a saved batch, not an individual keystroke or an identified person's
change. `recorded_by` names the flusher; background flushes may say `docsync` and
merged batches may include several authors. Named checkpoints record the authenticated
actor and user who named the agreement. The activity trace remains available for
sparse authored/reviewed actions; its text truncation does not affect versions.

## Retention and upgrades

History starts when this code is installed and a specification is next saved.
For an existing specification, the first archive entry is its available baseline,
labelled with the time it was captured, **not an invented historical edit time**.
A read does not seed history. Naming an unchanged legacy specification can capture
its baseline, provided its canonical CRDT log exists. Already compacted past
states cannot be reconstructed.

The live log remains compactable. History keeps its own deltas and materializes a
full CRDT state every 64 saves, limiting reconstruction to at most 63 later deltas.
All historical versions are retained; storage grows with edits, and backups must
include the SQLite database. Deleting the specification/project cascades its
versions and checkpoints. This is not an offsite backup or a tamper-proof audit log.

## API

- `GET /v1/mindmaps/{id}/versions?limit=30&before=64&checkpoints=false` lists newest
  first. Follow `next_cursor` as `before`; `total` counts all matching versions.
- `GET /v1/mindmaps/{id}/versions/{version}` returns the full earlier section tree
  and relationships. `notes` is untruncated plain text; `prose_xml` retains rich
  formatting for inspection; `prose_structure` gives canonical structured content
  for comparisons independent of XML attribute order. The UI reads plain text and exposes other changed
  fields in expandable details.
- `GET /v1/mindmaps/{id}/versions/{version}/state` downloads a Yjs v1 update that
  reconstructs the version in an **empty** Y.Doc, including rich content.
- `POST /v1/mindmaps/{id}/checkpoints` with `{ "name": "Agreed scope",
  "expected_version": 64 }` names that exact current saved version. Wait for
  durability acknowledgement, read `head`, then submit it. If another save wins,
  refresh and review before retrying. Unsaved/offline work is not part of a
  checkpoint. A version allows at most 16 names. Names cannot move to other versions; an exact retry is harmless
  while that version remains the saved head.

All routes enforce project scope. Reads require `read`; creating a checkpoint
requires `write` and an unarchived project. Neither viewing nor downloading an
old version mutates the live CRDT.

## Restoration

Live restoration is deliberately not exposed in this change. Applying an old
Yjs update to the live document merges it; it does **not** reverse deletions or
restore a previous state. A future restore operation must create new CRDT edits,
preserve rich fragments and section identities, guard against intervening edits,
and leave both the original version and the pre-restore version available.
The downloaded state already provides a complete recoverable copy for tooling.

The hosted MCP endpoint offers `takomo_specification_history`,
`takomo_specification_version`, and `takomo_specification_checkpoint` with the
same authorization and saved-head rules.
