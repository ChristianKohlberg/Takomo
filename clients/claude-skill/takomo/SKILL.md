---
name: takomo
description: Use the central Takomo as the single source of truth for work items instead of TodoWrite/todo lists. Use when a TAKOMO_URL + TAKOMO_TOKEN are configured for this session; covers finding work, claiming, progressing tickets through their workflow, and attaching evidence.
---

# Takomo client

The Takomo at `$TAKOMO_URL` is the single source of truth for work — a hosted, shared task tracker. Do not keep a private todo list for anything another agent, tool, or a human might need to see — put it in the store.

## Use the `takomo` CLI

The primary interface is the `takomo` command (`clients/cli/`). It wraps the REST API, remembers your claim's fence locally, and prints every store error legibly. Configure it once:

```bash
export TAKOMO_URL="https://host/v1"   # note the /v1
export TAKOMO_TOKEN="tk_..."
export TAKOMO_PROJECT="<proj>"        # optional default project
```

The common flow:

```bash
takomo new --type task "Wire up the frobnicator"   # create (warns on possible duplicates)
takomo ready                                        # what's claimable right now
takomo next                                         # atomically claim the next ready ticket
takomo start <id>                                   # claim if needed + move to in_progress
takomo comment <id> "opened PR, waiting on CI"      # narrate progress
takomo link <id> --pr <url> --branch <b>            # attach evidence
takomo done <id>                                    # finish (claim auto-releases)
takomo show <id>                                     # full ticket incl. comments/deps
takomo ls -q frobnicator                            # search
```

Run `takomo help` for every verb. After a claim, `takomo` echoes the lease fence automatically on `start`/`done`/`comment`/`link`, so you never pass it by hand.

## The state machine

Tickets move through their project's workflow via transitions only — there is no status field to set. The plain-tracker (`simple`) workflow is: `draft` → `todo` → `in_progress` → `done`, with `blocked` and `cancelled` as escape hatches. `todo` is claimable (it feeds the ready queue). Claiming a ticket is the only gate to move it — there are no human-approval gates in this workflow.

A `4xx`/`409` is a teaching response, not a failure: it lists `allowed_transitions` and what each requires (usually just `claim`). `takomo` prints these for you. Never retry a rejected call unchanged — read the remedy and adjust.

## Claiming is how you get work

Get work by claiming, never by browsing: `takomo next` (or `takomo claim <id>`) hands you a ticket with a lease. While you hold it, you may progress and edit it; if a call ever returns `fence.stale`, your lease expired and the ticket may belong to someone else — **stop writing to it** and re-claim or move on. If you stop without finishing, `takomo release <id>` returns it to the queue.

## Writing to tickets

Prefer commutative writes — they never conflict:
- Comments for narrative: `takomo comment <id> "..."` — progress, findings, questions.
- `takomo link <id> --pr/--branch/--env/--run <url>` — attach evidence as soon as it exists.
- Custom data → `PATCH /v1/tickets/<id>` with `metadata_merge` under your own namespace.

Only whole-`body` replacement needs the CAS dance (GET, send `If-Match: "<version>"`, retry on `conflict.version`). If you do that often, you should be commenting instead.

## Ask a human when you are blocked on a decision

When progressing needs a human judgment you cannot make — a confirmation ("OK to drop this table?"), a choice between options, a clarification, or an approval — do not guess and do not silently stall. Ask: `takomo ask <id> --title "..." --kind confirm|choose|clarify|approve [--option ...] [--expertise domain:billing] [--recommend "..."]`. This parks the ticket in a blocked state, releases your lease, and posts the question to the ask-a-human board. **Then end your run** — this is block-and-resume, not a wait. When you (or the next worker) pick the ticket back up, `takomo show <id>` carries the human's answer (also on the board / `takomo questions`); the answer resumes the ticket into a claimable state. Route to the right person with `--expertise` tags; set `--expires-in`/`--on-timeout` if the work has a deadline. Withdraw with `takomo withdraw <qid>` if you no longer need the answer.

## Creating tickets

Always search first (`takomo ls -q <keywords>`). `takomo new` auto-sends an Idempotency-Key and surfaces a `similar` list in the response — **read it**; if your ticket already exists, use the existing one. Structure work with `epic` parents grouping `task`/`bug` children (`--parent <epic-id>`); real dependencies are `blocked_by` edges (`takomo dep <id> --blocked-by <other>`), not prose.

## Raw HTTP (for non-`takomo` clients)

Under the hood every call is `curl -sS -H "Authorization: Bearer $TAKOMO_TOKEN" -H "Content-Type: application/json"` against `$TAKOMO_URL`. Every error body is self-describing: read `message`, `remedy`, and (on transitions) `allowed_transitions`.

**WAF note:** if you write your own client, set a custom `User-Agent` header. Some deployments sit behind a WAF that blocks the default `python-urllib` UA (curl's default UA passes fine). With `urllib`, add `Request(url, headers={"User-Agent": "takomo-client/1", ...})`; with `requests`, it is already fine.

## Rules

1. Claim before you work; release (`takomo release <id>`) if you stop without finishing so the ticket returns to the queue.
2. Never work around a workflow rejection — it encodes a project decision. Read `allowed_transitions` and move a legal way.
3. One ticket = one deliverable. Split with child tickets rather than letting scope creep.
4. Attach evidence links the moment they exist; a ticket without links is an unverifiable claim.
5. On any `429`, honor `Retry-After` and slow your loop — you are hammering the store.
