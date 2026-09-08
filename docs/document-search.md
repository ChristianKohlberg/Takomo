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
than paying to embed unchanged content again. The status reports queued, running
and indexed sections and a sanitized failure message. Failed provider calls retry
with bounded backoff; keyword search remains available.

API routes and permissions are described in `spec/openapi.yaml`. Search/status
require read access to the map's project. Manual sync also requires write access
and an active project. Global configuration requires unrestricted admin access.
