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
delays are configurable; no provider is contacted until a key is supplied.
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
runs transactionally and preserves existing source tables, IDs and links. Existing
maps are scheduled for projection on startup; unconfigured instances still get
keyword search without an external service.

A section is the normal chunk. Long sections split at paragraph boundaries around
2000 characters, splitting an oversized paragraph only as a last resort. Ancestor
headings accompany each chunk. Search presents the best matching chunk per section.

Saved content changes mark maps durably for projection. Only sections whose content
or heading path changed are queued, with a default 60-second quiet delay capped at
five minutes from the first pending change. The worker checks every five seconds;
provider capacity and retry backoff can add time after a job becomes eligible.
Unrelated edits do not reset a section's deadline. Leases recover interrupted jobs,
and content hashes plus provider fingerprints reject stale completions.

**Sync embeddings** in Document flushes pending document saves and makes the map's
pending jobs eligible immediately. It preserves already current embeddings, rather
than paying to embed unchanged content again. The status reports queued, running,
failed and indexed sections and a sanitized failure message. A failed provider
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
map has changed since the last projection, so a modal polling status or a
person pausing between keystrokes does not serialise behind claims or push a
refresh to open project sockets. A query with no word in it (punctuation, an
emoji) has nothing to embed and spends neither budget nor provider call.

API routes and permissions are described in `spec/openapi.yaml`. Search/status
require read access to the map's project. Manual sync also requires write access
and an active project. Global configuration requires unrestricted admin access.
