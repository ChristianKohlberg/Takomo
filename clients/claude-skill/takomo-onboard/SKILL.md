---
name: takomo-onboard
description: Onboard the current git repo onto the central Takomo so this and future sessions track work in the shared store instead of an ad-hoc todo list. Use once per repo when a Takomo URL + an admin token are available and the repo has no .takomo/ config yet.
---

# Onboard this repo onto Takomo

Goal: make the current repository a first-class project on the shared takomo in one command, then wire it so every future session in this repo uses the store automatically. Do this once per repo.

You do NOT re-implement any of the provisioning — `takomo init` owns creating the project, applying the workflow, minting a scoped agent token, and writing repo-local config. Your job is to run it, verify it, and leave the repo wired.

## Preconditions

- You are at the root of a git repository (`git rev-parse --show-toplevel` succeeds).
- The `takomo` CLI is on `PATH` (see `clients/cli/README.md` — it is a symlink one-liner). Run `takomo help` to confirm.
- An **admin-scoped** token is available for provisioning. Export it just for the init step:
  ```bash
  export TAKOMO_URL="https://your-store/v1"   # the /v1 base
  export TAKOMO_TOKEN="tk_<admin token>"       # admin scope, used ONLY to provision
  ```
  If you do not have an admin token, stop and ask the human for one — do not attempt to mint without it.

## Step 1 — run `takomo init`

From the repo root:

```bash
takomo init                 # derives the project id from the repo directory name
# or pin it explicitly / choose a workflow:
takomo init myproject --workflow simple
```

This will, using the admin token: create the project if missing and apply the `simple` plain-tracker workflow, mint a `read,write` agent token scoped to just this project, and write `.takomo/config` (url + project) and `.takomo/token` (the agent token, mode 600, auto-gitignored). It finishes by running a verification (`takomo ready`).

If it reports a `404` on token minting, the store is running an older build without the token endpoints — tell the human the store needs the current build deployed before onboarding.

## Step 2 — verify

Confirm the repo is wired and the agent token (not your admin token) works. Unset the admin env vars first so you are exercising the written config, not the admin token:

```bash
unset TAKOMO_TOKEN TAKOMO_URL          # fall back to .takomo/ config
takomo whoami                                    # should show actor agent:<project>, scopes read,write
takomo new "onboarding smoke test"               # create
takomo ready                                     # it shows up
ID=$(takomo next | awk '{print $2}')             # claim it
takomo done "$ID"                                # finish
```

`takomo` walks up from the cwd to find `.takomo/config`, so these work from anywhere in the repo with no environment set up.

## Step 3 — wire future sessions

So later sessions use the store instead of a private todo list:

1. Commit `.takomo/config` (the URL + project id are shared, not secret). Confirm `.takomo/token` is gitignored — `takomo init` adds it, but double-check `git status` does not show the token.
2. Install/enable the runtime `takomo` skill (`clients/claude-skill/takomo/SKILL.md`) for this repo so agents claim and progress real tickets. Point the project's agent instructions (e.g. `AGENTS.md`/prime) at the store as the source of truth for work items.

## Rules

- Never commit `.takomo/token`. If `git status` shows it, add `.takomo/token` to `.gitignore` and remove it from the index before committing anything.
- The admin token is only for the `takomo init` step. Everyday work uses the scoped agent token in `.takomo/token`.
- One project per repo. If `takomo init` says the project already exists, that is fine — it just refreshes the workflow and mints a fresh agent token.
