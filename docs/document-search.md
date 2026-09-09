# Document discovery search

In Document, use **Search** or **Cmd+S / Ctrl+S** to search the current project's
specification. The modal combines keyword matches and related meaning when an
embedding provider is configured. Arrow keys select a result; Enter opens its
section and selects the original passage when it is still present. Escape closes
the modal. The separate Find command remains literal next/previous occurrence
search, and browser Cmd+F is unchanged.

Results are grouped by section and show the heading path and an original text
excerpt. Highlights identify literal query words in that excerpt; a result found
only by its vector is labelled related meaning without invented highlights.
Keyword search continues to work when embeddings are unconfigured or unavailable.

A response carries at most 20 sections and says so: `limit`, `candidates` (the
distinct sections among the bounded candidate set of the top 100 keyword and top
100 semantic chunks), `truncated`, and a `note` when sections were left out. The
candidate count is not a count of every section that could match, so the modal
shows "top 20 of N" rather than a total. Query embeddings are bounded to 60 per
token per minute; past that the search answers from keywords alone and reports
`semantic_status: throttled` instead of failing or spending more.

## Provider configuration

An unrestricted administrator can configure embeddings in Settings. The default
is Voyage, `voyage-4-lite`, 1024 dimensions. The other supported wire protocol is
an OpenAI-compatible embeddings endpoint. Endpoint, model, dimensions and indexing
delays are configurable; no provider is contacted until a key is supplied. The
settings PUT replaces the configuration whole: all six fields travel together,
and a body that omits one is refused before anything changes, so a key cannot be
rotated by sending the key alone. Only the key may be omitted, which keeps it.
Use a provider that supports the selected dimensions and request format.

API keys are write-only in the settings API and stored in the server's SQLite
configuration, so protect database files and backups as credentials. Omitting a
key retains it when provider and endpoint are unchanged. Changing either clears
the old key unless a replacement is supplied; an empty key disables embeddings.
Provider responses and authentication headers are never included in index errors.

Changing provider, endpoint, model or dimensions creates a new vector fingerprint
and queues existing sections again. Search never compares vectors from different
fingerprints. Existing document content is unchanged. Only an administrator should
choose endpoints: document text and search queries are sent to the configured
provider. Local testing uses a mock endpoint, never production content.

## Indexing and compatibility

The CRDT update log and document structure remain authoritative. SQLite FTS5,
chunks, vectors and queue records are additive, rebuildable projections. Migration
runs transactionally and preserves existing source tables, IDs and links. The first
upgrade schedules every existing map for projection; an ordinary restart schedules
nothing, because the trigger already marks a map when its log grows and a
projected map is projected. Unconfigured instances still get keyword search
without an external service.

A section is the normal chunk. Long sections split at paragraph boundaries around
2000 characters, splitting an oversized paragraph only as a last resort. Ancestor
headings accompany each chunk. Search presents the best matching chunk per section.

Saved content changes mark maps durably for projection. Projection reads the
map's log on a read connection and applies the result in one short write that
first rechecks the log's sequence; if the log grew meanwhile the projection is
recomputed, a bounded number of times, and a map still changing under it stays
marked and answers reads as `stale` until the next pass completes. Only sections
whose content or heading path changed are queued, with a default 60-second quiet
delay capped at five minutes from the first pending change. That window belongs to
one pending change: a section whose job was parked by failures, or whose window
already elapsed while it waited in a backlog, starts a fresh quiet period when it
is edited again rather than becoming due at once. The worker checks every five
seconds; provider capacity and retry backoff can add time after a job becomes
eligible. Unrelated edits do not reset a section's deadline. Leases recover
interrupted jobs, and content hashes plus provider fingerprints reject stale
completions. A completion that arrives while the map is still changing is
discarded and its job waits one more quiet period, so the same text is not sent
to the provider again in the same pass; a projection failure at that point is
treated the same way and the other jobs in the pass continue.

The embedding-status icon in the Document ribbon opens a compact status dialog. It
shows stored/current passage embeddings separately from pending, running and failed
section jobs; long sections can contain several passages. Completion requires a
current projection, no pending jobs/errors, and every passage embedded for the
configured provider. Edits the server has not yet acknowledged over the sync socket
also prevent the completed indicator; that judgement comes from the durability
reply alone, so a failed local draft replica does not pin the icon on pending. The
page reads status every 20 s while no dialog is open, every 3 s while one is, not
at all while the tab is hidden, and once immediately when a dialog opens, the tab
returns, or the server acknowledges edits that had held the icon on pending. An
edit becoming pending does not trigger a read on its own.

**Last successful sync** is historical: it records the last actual accepted provider
completion that left this map fully indexed for the current provider fingerprint.
Status polling and unchanged manual sync never advance it. Existing databases and
empty maps show no recorded time until an actual full completion; changes may make
the index pending while the previous successful time remains visible. The metadata
is additive and preserved across restart. Changing provider identity does not
borrow another model's completion time.

**Embed now** in this dialog (or **Sync document** in search) flushes pending document saves and makes the map's
pending jobs eligible immediately. It preserves already current embeddings, rather
than paying to embed unchanged content again. The response says whether that
happened: `sync: scheduled`, or `sync: deferred` with a `sync_note` when the
document kept changing under every projection attempt. Deferred means nothing was
scheduled and pending changes keep their normal quiet delay; the modal shows the
note until the index reports current, and syncing again once editing pauses
applies the bypass. `EmbeddingStatus` in `spec/openapi.yaml` defines every status
field: section job counts, passage embedding counts, the last recorded completion
and a sanitized failure message. A failed provider
call retries with bounded backoff at most three times; after that the job is
parked and counted as `failed`, and is not sent to the provider again until the
section's content changes, the provider configuration changes, or a manual sync
resets it. Keyword search remains available throughout.

A map whose source log cannot be replayed is isolated: its failure is recorded
and shown as `last_error` in its status, its projection is left as it was, and
every other map keeps indexing. Search and status keep answering from that last
good projection (empty, if there never was one) and say so: both carry
`projection: stale` and the search response its `projection_error`, from the
very first read after the failure. The next content change or a manual sync
retries it. A manual sync on an archived project is refused with the same
`project.archived` contract every other project write meets.

A clean read costs no write: search and status take the writer only when the
map has changed since the last projection, so a page polling status or a
person pausing between keystrokes does not serialise behind claims or push a
refresh to open project sockets. A query with no word in it (punctuation, an
emoji) has nothing to embed and spends neither budget nor provider call.

API routes and permissions are described in `spec/openapi.yaml`. Search/status
require read access to the map's project. Manual sync also requires write access
and an active project. Global configuration requires unrestricted admin access.
