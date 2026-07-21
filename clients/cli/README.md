# `takomo` — takomo command-line client

A single self-contained CLI over the Takomo REST API — the beads/beans-style
surface for creating, searching, claiming, and progressing tickets without
hand-writing curl. It is `bash` + `curl` + `python3` (stdlib only); curl's
default User-Agent sails past WAFs that block python-urllib, and python is used
only locally to build request bodies and format output.

## Install

One command — `install.sh` puts `takomo` on your `PATH` (symlinking a checkout copy,
or downloading it standalone) after checking `curl` and `python3` are present:

```sh
# from a checkout (symlinks the repo copy, so `git pull` keeps takomo current):
./clients/cli/install.sh

# standalone (no checkout):
curl -fsSL https://raw.githubusercontent.com/ChristianKohlberg/Takomo/main/clients/cli/install.sh | sh
```

Knobs: `TAKOMO_BIN_DIR` (default `~/.local/bin`), `TAKOMO_REF` (git ref to
fetch, default `main`), `TAKOMO_CLI_URL` (override the raw `takomo` URL).

Or symlink it by hand:

```sh
ln -s "$PWD/clients/cli/takomo" ~/.local/bin/takomo   # or anywhere on $PATH
ln -s takomo ~/.local/bin/tk                           # optional short alias
takomo help
```

## Configure

The fastest path is `takomo init` (see below) — it writes a repo-local `.takomo/`
config so nothing needs exporting. Otherwise, environment variables:

```sh
export TAKOMO_URL="https://your-host/v1"   # required (note the /v1)
export TAKOMO_TOKEN="tk_..."               # required (read/write token)
export TAKOMO_PROJECT="myproj"             # optional default project
export TAKOMO_ACTOR="me"                   # optional, only for `ls --mine`
```

### Repo-local config via `takomo init`

Inside a git repo, with an admin token in `TAKOMO_TOKEN`:

```sh
takomo init                    # or: takomo init myproject --workflow simple
```

This creates/prepares the project, mints a `read,write` agent token scoped to it,
and writes `.takomo/config` (`url`, `project` — safe to commit) plus
`.takomo/token` (mode 600, auto-gitignored). Afterwards `takomo` auto-loads
`.takomo/` by walking up from your cwd, so `takomo new`/`takomo ready`/etc. work with
no environment set. Precedence is **explicit flag > environment > `.takomo`**.

`takomo whoami` prints the calling token's actor, scopes, and projects. Admins can
manage tokens over HTTP with `takomo token create|ls|revoke`, and delete a whole
project with `takomo project rm ID [--force]`:

```sh
# Cascade-delete a project and ALL its tickets, comments, deps, and events.
# Prints a pre-delete summary (ticket counts by state) first.
takomo project rm myproj
# Refused with a 409 if any ticket has an active claim — pass --force to delete
# anyway (this abandons those live leases):
takomo project rm myproj --force
```

Deleting a project does not remove tokens scoped to it; they just stop resolving
once it is gone. Revoke them separately with `takomo token revoke ID`.

After you `claim`/`next` a ticket, its fence (the lease token every mutating
call must echo) is remembered locally under
`${XDG_STATE_HOME:-~/.local/state}/takomo/`, keyed by store URL + ticket id —
so `start`, `done`, `comment`, `link`, and `dep` just work without you passing
`--fence`. Every store error is printed legibly (`message`, `remedy`, and for
transitions the `allowed_transitions`).

## Worked example

```sh
# create a ticket (prints its id; warns about possible duplicates)
takomo new --project myproj --type task --priority high "Wire up the frobnicator"
#   created myproj-a1b2  [todo]  Wire up the frobnicator

# see the queue, then grab the next ready ticket (fence saved locally)
takomo ready --project myproj
takomo next  --project myproj
#   claimed myproj-a1b2  [todo]  Wire up the frobnicator  (fence 1 saved)

# start work, attach evidence, leave a note
takomo start   myproj-a1b2                 # -> in_progress
takomo link    myproj-a1b2 --pr https://github.com/org/repo/pull/42 --branch feat/frob
takomo comment myproj-a1b2 "Opened PR, waiting on CI"

# finish it (fence auto-included; claim auto-released on terminal)
takomo done myproj-a1b2                     # -> done

# search / inspect
takomo ls   --project myproj --state in_progress
takomo ls   --project myproj -q frobnicator
takomo show myproj-a1b2
```

If a move is illegal for the project's workflow, `takomo` prints the store's
teaching 409 — the allowed transitions and what each requires — instead of a
bare error, e.g.:

```
$ takomo done some-todo-ticket
HTTP 409  [transition.illegal]
No transition from 'todo' to 'done' is defined in this workflow.
current state: todo
  allowed -> in_progress (requires claim)
  allowed -> cancelled
```

## Portability & observability

```sh
# Export a project's tickets (with comments and dependency edges) as JSONL:
takomo export --project myproj --out backup.jsonl     # or to stdout without --out

# Import data in. Idempotency-Key = the origin id, so re-import is SAFE
# (already-present tickets replay rather than duplicating):
takomo import --from takomo backup.jsonl --project myproj   # round-trips takomo export
takomo import --from beads issues.jsonl     --project myproj   # beads JSONL
takomo import --from beans ./repo           --project myproj   # a repo (its .beans/ dir)

# Tail the event log, printing new events as they land:
takomo watch --project myproj                # or --since N to start from a cursor

# Store metrics: ticket counts by state/category per project, claims, events:
takomo metrics
```

Import maps each external item onto the target project's workflow: title, body,
status→state, priority, tags→labels, parent, and dependency edges, with the
origin id and status preserved under `metadata.import.*`. Status is applied
best-effort by driving the workflow (claiming where an edge requires it); a
ticket that can't be moved (a gate the token lacks, or an open blocker) stays in
the initial state with its origin status recorded in metadata. `beans` imports
need PyYAML (`pip install pyyaml`) to read the Markdown frontmatter.

## Command summary

Run `takomo help` for the full list. Onboarding & identity: `init`, `whoami`,
`token create|ls|revoke`. Work verbs: `new`, `ls`, `ready`, `show`, `claim`,
`next`, `start`, `done`, `block`, `cancel`, `move`, `comment`, `link`, `dep`,
`release`. Portability & observability: `export`, `import`, `watch`, `metrics`.
