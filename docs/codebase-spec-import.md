# Codebase specification import: development status

The import is being built in milestones; see the [implementation plan](plans/codebase-spec-import.md).
The first slice provides the shared scoped repository reader and an inventory-only
preflight command. It does **not** generate specification prose, create import jobs,
or change documents yet. The scope contract is enforced by the same repository
module used by the agent service, ready for the forthcoming import job kind.
The existing research HTTP API has not gained new scope fields in this slice.

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
   tested input is reproducible. Actual scoped import inference is a later milestone.
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

The scope and preflight foundation is implemented. Still pending: canonical
publication transaction proof; durable import/task/evidence storage and APIs;
job capability and budget integration; structured outline/draft/verification
stages; review UI and atomic acceptance; quality evaluation and rollout. A scoped
run must retain its scope and limits in every task snapshot and display partial
coverage in the final draft. Neither an agent tool call nor a retry may expand it.
