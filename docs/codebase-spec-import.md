# Codebase specification import: development status

The MVP generates a bounded draft with our existing Codex App Server adapter,
then imports it as ordinary, unreviewed sections into an empty Takomo specification.
Start through the project wizard with GitHub or through the local CLI below. The **existing document and mindmap are the review UI**: read,
edit, move and confirm sections using their normal controls. There is no separate
staging dashboard and no new review data model.

## Generate a small draft, then review it in Takomo

Use the agent service's dedicated authenticated Codex home (see
`services/agent/README.md`). This command makes **one real App Server turn**, with
a three-minute turn deadline, at most 100 files / 1 MB / 50 repository calls /
12 sections. Defaults are 30 calls and 6 sections. No code is executed.

```sh
node services/agent/spec-import-cli.mjs generate \
  --repo . \
  --include services/agent/repository-scope.mjs \
  --max-files 1 --max-bytes 20000 --max-tool-calls 8 --max-sections 3 \
  --out /tmp/scope-spec-draft.json
```

The file is reserved before inference and records status, pinned scope/revision,
thread/turn IDs, the draft and inspected source ranges. Existing output files are
refused. Failed runs stay failed; rerunning generation requires a new output file
and is a new model call. Source ranges must have been read within this run, but
that mechanical check does not prove the prose is correct. This is an as-built
review draft, not validated product intent or a claim that tests passed.

To load a ready draft, set `TAKOMO_URL` and `TAKOMO_IMPORT_TOKEN` with a
project-restricted human/write token, then pass the ID of an empty specification:

```sh
node services/agent/spec-import-cli.mjs publish \
  --file /tmp/scope-spec-draft.json --mindmap mm-YOURID
```

Publication makes **no model call**. It creates a root describing scope and open
questions, plus the generated parent/child sections with plain paragraphs and
source references. All remain unconfirmed. Open the project's normal Document or
Map view to review them. Existing text is never replaced. Retry the identical
artifact against the same target and actor after a lost response: the stored
receipt returns the original node mapping without duplicating content or undoing
human edits. Changed payloads with the same request ID conflict.

`POST /v1/mindmaps/{id}/codebase-import` requires human/write and project access.
It checks shape, source scope and an empty target under the room lock, builds on
a private replica, and commits the tree plus receipt as one CRDT update before
exposing it to peers. A persistence failure leaves the live tree unchanged. The
receipt is part of the document, so saved versions include it. The server does
not independently inspect the repository; direct API callers are responsible for
their source claims, as with other human-authorized document edits.

MVP limits: plain paragraphs and a single bounded generation turn. The CLI uses
local artifacts and explicit publication; the GitHub wizard queues one explicitly
authorized run and publishes its result. Multi-task resume, progressive per-section
generation, semantic indexing and refresh of existing content are not shipped yet. The broader [implementation plan](plans/codebase-spec-import.md)
remains the roadmap, amended to reuse the document/map instead of a staging UI.

## Test a small part before spending inference

Run from the repository root with Node 22+ and Git. No Codex login, provider key,
server, network connection, or npm install is needed:

```sh
node services/agent/spec-import-preflight.mjs \
  --repo . \
  --include services/agent \
  --exclude services/agent/test \
  --max-files 20 \
  --max-bytes 150000
```

For one source file:

```sh
node services/agent/spec-import-preflight.mjs \
  --repo . \
  --include src/store/mindmapdoc.rs \
  --max-files 1
```

`--include` and `--exclude` can be repeated. They are literal repository-relative
files or directory prefixes, not globs; `src/api` includes its descendants but
not `src/api_extra`. Exclusion wins. Traversal, absolute paths, backslashes,
trailing slashes and control characters are rejected. Omit a trailing slash on
folder names. A typo in an include or an empty eligible selection fails visibly.
Scope never grows to follow an import or a cross-package dependency. Later
analysis must report those dependencies as outside scope until a user expands it.

`--include` is required even for a whole repository (`--include .`). Development
defaults are 100 eligible files, 1,000,000 source bytes and a recorded 30-tool-call
ceiling for later analysis. `--max-files`, `--max-bytes` and `--max-tool-calls`
allow deliberate changes within the reader's hard ceilings. Preflight does not
claim to enforce token/currency spending: it launches no model. Future inference
runs also need task/run token, time and result-size budgets.

Preflight resolves `--revision` (default `HEAD`) once, reads committed Git objects,
and emits JSON with the exact commit, normalized scope, deterministic inventory
id, limits, complete selected-path counts and up to 200 eligible filenames.
`files_truncated` means the filename preview is partial, not that files were sampled
from the inventory. Dirty/untracked files are excluded; commit a test fixture or
change first if it must be visible. `--out /tmp/preflight.json` also saves the JSON
without overwriting an existing file. It is an inventory artifact, not a resumable
import checkpoint; it does not retain Git objects against garbage collection yet.

Counts distinguish excluded paths, nonregular entries (including symlinks and
submodules), unsupported paths and files over 1 MB. Binary content is detected
only during reads/searches, so source-byte counts may include binary files.
Counts cover the included paths; preflight does not scan outside that selection
just to compute a whole-repository denominator. Metadata bytes are not input-token
estimates, and listing files is not behavioral coverage. No automatic sensitive-file
or generated-file classification is implemented yet; explicitly exclude those paths.

## Development testing layers

1. Use disposable, committed fixture repositories and the fake App Server for
   the ordinary development loop. No paid inference is involved.
2. Run preflight on one real package/file with small ceilings before opting into
   a real provider smoke. Reuse its exact commit and scope in that smoke so the
   tested input is reproducible. Use the generation command above for a deliberately small real run.
3. Expand to representative services and monorepos only for quality/scale
   evaluation. Full-repository runs are not a prerequisite for editing the importer.

Focused checks:

```sh
node --test services/agent/test/repository-scope.test.mjs services/agent/test/research.test.mjs
node --test services/agent/test/*.test.mjs
```

Tests cover scope enforcement across all repository tools, exclusions, prefix
boundaries, literal Git metacharacters, dirty-file isolation, pagination above
200 files/100 matches, streamed search exceeding the former 2 MB cap, UTF-8 read
continuations, oversized/binary files, invalid cursors, resource limits, and CLI
output preservation. Existing live-provider tests remain opt-in.

## Next milestones

Earlier local-CLI preview validation: the full debug Rust suite passed (657 tests), the agent
suite passed (83 tests; five provider tests skipped), Clippy and formatting passed,
and the frontend built. A disposable HTTP import was opened in both existing
Document and Map views, showing the same nested sections and source notes. The
browser content was an explicitly labeled fixture, not real model output. A real
provider quality check and the integration/release validation gate remain pending;
this branch has not been merged or deployed.

The local generation and durable empty-document import MVP is implemented. Still
pending: durable multi-task progress, separate verification/reconciliation stages,
quality evaluation and rollout. The GitHub wizard has a durable single-turn queue. A scoped
run must retain its scope and limits in every task snapshot and display partial
coverage in the final draft. Neither an agent tool call nor a retry may expand it.

## Project wizard and GitHub follow-up

The worktree now also contains project creation from the nav search, GitHub App
connection settings, project repository settings and a bounded extraction queue.
See [GitHub setup and extraction](github-extraction.md). This extends the earlier
local-CLI milestone; GitHub setup and live provider validation remain separate
from the fixture-backed local checks described above.
