# GitHub project setup and specification extraction

Create a project from **Create new project** at the end of the nav rail's project
search results. The entry is keyboard-accessible and carries the search text into
the wizard. The same wizard opens from Settings → Projects.

The wizard explains the draft, asks whether to connect GitHub, lets you select an
accessible repository and a literal file/folder, and then asks explicitly whether
to start extraction. Connecting alone never starts a model. You can finish with
an empty project, save a connection for later, or start one small run.

Review uses the existing **Document and Mindmap**. Sections are ordinary agent
content with no human confirmation. Sources, the exact commit, limited scope and
open questions stay in the document. Project settings show the run status and
provide links to both views. They also let you change the repository/scope and
start later. An existing specification is never replaced.

## Operator setup: dedicated GitHub App

This MVP uses one **deployment-owned GitHub App**, not a user's personal access
token. It is intended for a self-hosted instance whose unrestricted administrators
are trusted with every connected repository. A project-scoped administrator may
create an empty project but cannot enumerate or connect deployment-wide private
repositories. Per-user GitHub OAuth/account linking is a separate future feature.

Register a dedicated GitHub App for this Takomo deployment:

- Repository permission: **Contents: read-only** (Metadata read is implicit).
- No write permissions, user authorization flow or webhook is needed for this
  polling MVP. Do not configure an installation callback to a localhost preview.
- Install for **selected repositories**. Each installation is explicitly connected
  to Takomo by an unrestricted human administrator in Settings → GitHub.
- Keep the generated RSA private key in a server-only file; configure:
  `TAKOMO_GITHUB_APP_ID`, `TAKOMO_GITHUB_APP_SLUG`, and
  `TAKOMO_GITHUB_PRIVATE_KEY_FILE`. Restart the server after changing its environment.
  The key is never stored in the database, browser, job payload or model context.

On Render, add the PEM as a Secret File and set `TAKOMO_GITHUB_PRIVATE_KEY_FILE`
to `/etc/secrets/<exact-filename>`. The bundled container grants supplementary
group 1000 to the Takomo application (including admin commands), which permits
reading Render's runtime secret mounts. Kroki and Mermaid receive no supplementary
groups and cannot read these files. Do not make the key world-readable or change
the mounted file's ownership. Redeploy after updating the file or configuration.
See [Render secret-file permissions](https://render.com/docs/docker-secrets#accessing-secret-files-at-runtime).

Settings → GitHub shows whether configuration is present, opens the App's GitHub
installation page, refreshes installations and connects the selected account.
After granting access, return to Takomo and refresh. No inbound tunnel is required
for this flow. The installation list is bounded to 100; use a dedicated App.
Repository lists are paginated at 100 and refreshed against GitHub rather than
stored as a stale access allowlist.

**Manage repositories and permissions** opens GitHub's installation settings.
Repository selection and future permission upgrades require approval on GitHub;
Takomo cannot grant itself more access. The current importer requests only a
short-lived installation token with Contents read, restricted to the exact job
repository. Disconnecting locally fails pending runs and removes project links;
it preserves existing documents. Revoke the installation itself on GitHub to
withdraw the App's underlying access.

Reference: [GitHub App permissions](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/choosing-permissions-for-a-github-app),
[installation access tokens](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-an-installation-access-token-for-a-github-app).

## Source path checks

Saving a repository connection checks that each included file or folder exists
on the repository's default branch. Starting extraction repeats that check against
the exact commit captured in the job, so a saved path deleted later cannot queue a
model run. Missing paths, symlinks and submodules receive an error before saving
or enqueueing. Paths are literal and case-sensitive; spaces are not trimmed.

The check uses non-recursive [GitHub tree metadata](https://docs.github.com/en/rest/git/trees),
reads only ancestor directories, and caches shared ancestors within the request.
It permits at most 16 directory requests, a 15-second path-check deadline and the
existing 2 MB per-response limit. Incomplete listings are refused. This confirms
path existence and type; file counts, exclusions, binary content and source-byte
budgets are still checked by the worker before inference.

## Worker setup and bounded development

Run the existing agent service with its dedicated authenticated Codex home and
`agent:run` token scoped to the intended projects. Set `TAKOMO_GITHUB_IMPORTS=1`
to opt that worker into the extraction queue. Existing worker kinds keep their
current behavior. Without an enabled worker, the UI says the run is waiting.

The worker fetches the pinned commit and tree metadata into a disposable bare Git
repository with `--filter=blob:none`, without a working checkout or hooks. It
retrieves only selected regular-file blobs through GitHub, verifies each Git object
hash and runs the existing repository tools without lazy fetching. No repository
code is executed. Git credentials are never placed in command arguments, remote
URLs, artifacts or Codex inputs. Temporary repository objects are removed after
the attempt; the private draft artifact remains in the worker state directory.

One run allows **20 files, 100,000 source bytes, 12 repository calls and three
sections**, with a three-minute App Server turn deadline and five-minute job
deadline. Git metadata fetching has a one-minute deadline. Metadata transfer still
depends on repository size; source limits are not a currency/token spending cap.
A full-repository metadata index or semantic search is not implemented.

Claims expire after 60 seconds without heartbeat. Expired attempts fail and are
never automatically claimed again. Publication delivery can retry without another
model run. The durable document receipt prevents duplicate sections if publication
succeeds but its HTTP reply or subsequent queue update is lost. If an attempt is
reported interrupted, inspect the document before authorizing another run.

Start with this repository and scope **`examples/extraction-fixture`**, or just
`examples/extraction-fixture/checkout.mjs`. The README includes expected behavior
for human quality review. The fixture must be committed and present on GitHub
before GitHub-backed extraction can read it. A new external test repository is not
required.

## Validation and rollout

Use fake App Server responses and disposable repositories for ordinary tests.
GitHub credentials, a live provider call and a public tunnel are not needed to run
the local suites. Live GitHub installation/token exchange and provider quality
must be tested with a deliberately selected repository before rollout. This change
adds authentication, persistence and source retrieval; run the required integration and release
checks before merging or exposing it to users.

## Repeatable local smoke

Run the complete fixture path without GitHub credentials or paid inference:

```sh
CARGO_TARGET_DIR="$HOME/.cache/takomo-import-smoke" cargo test --locked --test spec_import fixture_app_server_to_document_smoke -- --nocapture
```

This uses Node 22 and a disposable committed copy of the checkout sample. The
real Codex adapter talks to a deterministic App Server protocol fixture, reads
only `examples/extraction-fixture/checkout.mjs`, and saves a simulated draft.
The test publishes through the real HTTP API into a temporary database, retries
the same artifact, and verifies the hierarchy and prose used by Document and
Mindmap. Its loopback server ends with the test process. It does not connect to
GitHub, run a live model, or evaluate specification quality.

To generate an inspectable artifact without starting any HTTP server:

```sh
node services/agent/test/import-fixture-smoke.mjs --out /tmp/checkout-simulated-draft.json
```

Choose a new output path for each attempt; existing artifacts are never overwritten.
In the wizard or project repository settings, **Use sample file** selects the same
single source file. Select Takomo (or a fork containing the fixture) first. The
shortcut does not save a connection or authorize extraction; launch remains an
explicit choice. GitHub-backed tests require the fixture to exist on the selected
repository's default branch. Live installation access and model quality remain
separate rollout checks.
