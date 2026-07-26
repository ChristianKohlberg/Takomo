# Ask a human

Sometimes an agent working a ticket hits a point it cannot decide alone: a
confirmation ("OK to drop this table?"), a choice between approaches, a
clarification, or an approval. The **ask-a-human board** is the channel for that
— a first-class question tied to a ticket, an inbox for the humans who answer,
and an answer that unblocks the work.

It is built entirely on primitives takomo already has: the `blocked`-category
state (`needs-decision` in the factory-default workflow), the `scope:human`
transition gates, the append-only event log, and per-token scopes.

## The flow (block-and-resume)

```
agent            takomo                         human
  │  takomo ask ───▶ park ticket (needs-decision)
  │                  release the agent's lease
  │                  record question (status: open) ──▶ inbox + optional push
  │  (end run)
  │                                              │  answer on /board (or API)
  │                  record answer ◀─────────────┘
  │                  human-gated transition → ready
  │  takomo next ──▶ re-claim, read the answer on the ticket, continue
```

Asking **parks and releases** — the agent does not hold a process open waiting.
It ends its run; an orchestrator (or the agent itself later) picks the ticket
back up once it is `ready` again. The answer is recorded on the ticket as a
comment and on the question, so `takomo show <id>` carries the decision.

## Raising a question

```sh
takomo ask rvp-x7k2 \
  --title "OK to drop table billing_v1?" \
  --kind confirm \
  --body "No reads in 90d; want a human to confirm before I migrate." \
  --expertise domain:billing \
  --urgency high \
  --recommend yes --rec-note "no reads in 90d" --confidence 4
```

### Make the question decision-ready

Beyond the required `title`/`kind`, a few optional fields let the inbox render a
richer, faster-to-answer card (all additive; omit any of them):

- **Per-option descriptions** — for `choose`, give each option a one-line
  trade-off. CLI: `--option X --option-desc "…"` (parallel, 1:1). Over
  MCP/HTTP: `options: [{ "value": "X", "desc": "…" }]` (or a parallel
  `option_notes` array).
- **`--recommend` + `--rec-note`** — your suggested answer and a short *why*.
- **`--confidence` 1–4** — how strong the recommendation is (1 tentative … 4
  very strong); drives the recommendation gauge.
- **`--summary`** — a one-line preview for the inbox list.
- **Multi-select** — a `choose` where several options apply: `--multi` (with
  optional repeatable `--rec-multi <opt>` for the suggested set; over MCP/HTTP
  `multi: true` + `recommended_multi`). The human ticks several; the answer is the
  chosen array.

The `ask` response includes a non-blocking **`hints`** array naming anything that
would make the card richer (e.g. "add a description per option", "add
confidence") — it never fails the ask, it just teaches the next one. The MCP
`takomo_ask` tool and its response carry the same fields and hints.

Kinds:

| kind      | answer control     | answer value      | who may answer                          |
|-----------|--------------------|-------------------|-----------------------------------------|
| `confirm` | Yes / No           | boolean           | any `human`-scoped token                |
| `choose`  | one of `--option`s | the option string | any `human`-scoped token                |
| `clarify` | free text          | the explanation   | any `human`-scoped token                |
| `approve` | Approve / Reject   | boolean           | **only a matching domain expert** (see below) |

`approve` is the strong gate: it *must* name at least one `--expertise` tag, and
only a token holding the matching `expert:<tag>` scope can answer it — a general
human is refused. Use `confirm` for a lightweight yes/no any human can make.

A ticket can carry **several open questions at once** — e.g. two decisions for
two different domain experts, asked in parallel before you end your run. The
ticket resumes **only when every open question on it is answered** (a barrier):
answering one while others remain records that decision and leaves the ticket
parked (`resolved_to: null`); the answer that clears the last one resumes it. A
re-sent `ask` with the same asker + kind + title is treated as an idempotent
retry (it returns the existing question, never a duplicate).

Over the API it is `POST /v1/questions` (needs the `write` scope); over MCP it
is `takomo_ask`. Echo your lease `fence` if you hold the ticket. `takomo show`
lists every open question on the ticket so a resuming agent sees the full set.

## Answering

A human answers on the dedicated **`/inbox`** page — an email-style triage
surface with a status folder rail (Open / Answered / Withdrawn / Expired), a
scannable question list, and a reading/answer pane (with a **mine** filter for
your expertise, inline answering, withdraw, and "create answer link"). A
**ticket filter** narrows the queue to one ticket's questions — the folder counts
follow it, and the choice is deep-linkable as `#ticket=<id>`, so "everything
still open on TKM-42" is a URL you can share. The `/board` also has a
lightweight **Ask a human** drawer with an unread badge for answering in
context; the board's own ticket filter narrows that drawer too, including the
selected ticket's subtasks. Or over the API:

```sh
takomo answer q-9f3ka2xz yes --note "confirmed with the data team"
```

Answering requires the **`human`** scope — it *is* the human authorization gate,
and it performs the ticket's human-gated resume transition (in the factory
default, `needs-decision → ready`). `POST /v1/questions/{id}/answer` / the
`takomo_answer` MCP tool are the other two surfaces.

An agent that no longer needs its answer withdraws it: `takomo withdraw <qid>`.

## Taking an answer back (reopen)

Answered by mistake? The inbox commits an answer only after a 30-second undo
window, but you can still **reopen** an answered question after that — a
conditional undo — *as long as the ticket hasn't started relying on the answer*.
`takomo reopen <qid>` (or the **Reopen** button on an answered question in
`/inbox`, or `POST /v1/questions/{id}/reopen`, needs the `human` scope) returns
the question to `open` and re-parks the ticket in a blocked state.

It is refused with a teaching `409` once the answer is in use: the resumed ticket
has been **claimed** (`question.reopen_claimed`), **moved on** past the state it
resumed into (`question.reopen_moved`), or **archived**
(`question.reopen_archived`). In that case, re-ask instead. Advisory questions
(which never changed ticket state) always reopen. Reopening an `approve` decision
needs the matching domain expert, just like answering it.

## Ask for more research before answering (follow-up thread)

A human doesn't have to answer immediately — they can **bounce the question back
to the asking agent for more research first**, and the exchange is tracked as a
clear follow-up thread on the question.

- In `/inbox`, the reading pane has an **Ask for more info** action. Sending a
  message records it on the thread and flips the question's `awaiting` from
  `human` to `agent`. The question **stays open** and, for a blocking question,
  the ticket **stays parked** — the human still owns the eventual answer.
- The request is mirrored onto the ticket, so the agent sees it on its normal
  work-loop (`takomo_show` surfaces the open question with its `thread`). The
  agent replies with `takomo_reply <qid> "<what you found>"` (or
  `POST /v1/questions/{id}/reply`), which flips `awaiting` back to `human`. A
  reply is only accepted when the question is actually awaiting the agent (a
  human bounced it back first); an unsolicited reply is refused.
- The inbox shows the thread inline and marks whose turn it is; the human then
  answers (or asks again). Any number of round-trips is fine.

API: `POST /v1/questions/{id}/followup {message}` (human scope) and
`POST /v1/questions/{id}/reply {message}` (write scope). `GET /v1/questions/{id}`
returns the question with its `thread` and current `awaiting`.

## Answer links — for outside experts (no standing token)

A teammate answers by pasting a `human`-scoped token into the board. For an
**outside** expert (a lawyer, a client) who shouldn't hold a standing token,
mint a **per-question answer link**: a scoped, expiring, **single-use** `tka_`
token that authorizes exactly one write — answering that one question — and
nothing else.

```sh
takomo answer-link q-9f3ka2xz --actor human:counsel@firm --ttl 172800
# → prints a  https://<host>/board#a=<token>  link (shown once)
```

You send them the link; they open it, see just that one question, and click an
answer. No login, no token to manage, no access to anything else. It expires
(default 3 days, max 30) and is spent after one answer. Revoke early with
`takomo answer-link revoke <grant-id>`.

You can only delegate authority you hold: minting a link for an `approve`
question requires you to hold the matching `expert:<tag>` scope. The link then
carries exactly the authority that one question needs — including satisfying the
approve gate — and is recorded under the `actor` you named. Over the API it is
`POST /v1/questions/{id}/answer-link` → `/v1/answer/self` (a distinct
answer-grant auth path); over MCP it is `takomo_answer_link`.

This is the recommended way to pull a decision from someone who isn't a takomo
user. (Notifications still deep-link to the board root, not a specific question —
put the `#a=` link in the message body you send the expert.)

## Blocking vs advisory questions

A question has a **mode**:

- **`blocking`** (default) — the flow above: it parks the ticket, releases the
  lease, and the ticket resumes when all its blocking questions are answered.
  Use it for a decision that must be made *before the work continues*. It
  requires a ticket in a state with a self-service edge into a blocked state.
- **`advisory`** (`--advisory` / `"mode": "advisory"`) — a routed, recorded
  decision that **changes no ticket state and touches no lease**. It can be
  asked on *any* ticket in *any* state (it behaves like a comment for write
  purposes — no fence needed), doesn't count toward the barrier, and answering
  it only records the decision (`resolved_to` is always null).

Advisory is the right fit for **epic-level and strategic questions** — "should
we do this epic at all?", "which direction for the whole feature?", "prioritize
A or B?". An epic is a container no agent claims/works, and parking it wouldn't
even block its children (readiness is graph-based, not parent-state-based), so a
*blocking* question on an epic makes little sense. An advisory one routes the
call to the right expert and records it without freezing anything.

```sh
takomo ask epic-42 --advisory --kind choose \
  --title "Rewrite or incremental for the billing epic?" \
  --option rewrite --option incremental --expertise domain:product
```

`on_timeout=cancel` is rejected for advisory (there's nothing to cancel on its
behalf); `recommended` on advisory just records the recommendation on expiry
with no state change, so it carries no minimum-window requirement.

## Routing by expertise

Questions carry `expertise` tags like `domain:billing`. Tags are **advisory**:
any `human`-scoped token may answer (a question is never stranded because its
expert is away), while the inbox and notifications route by tag.

A human "owns" a tag by holding the matching free-form scope `expert:<tag>`:

```sh
takomo token create --actor human:dana \
  --scopes read,write,human,expert:domain:billing
```

Then `takomo questions --mine` (or the board's **mine** toggle) shows only the
questions routed to that person's tags. No people/identity table is needed — it
rides the existing token scopes.

## Per-project conventions: language and style

Two project settings shape *how* agents write, not what they do. Both exist for
the same reason: to reach the **agent at the source**, so the text lands right
rather than being flagged after the fact. Both are **soft nudges** — surfaced,
never enforced — and both are project settings, so every agent and viewer sees
the same thing.

Either can be edited from the **Settings** sheet on `/board` (see
[On the board](#on-the-board) below) or from the CLI/API as shown here.

### Question language

A project can declare the human-facing language its questions should be written
in — e.g. **German** for a revamp project whose reviewers are German-speaking,
even though the agents and the underlying tickets work in English. Set it once
(admin):

```sh
takomo project language <project> German      # show current: omit the value
takomo project language <project> --clear
# over HTTP:  PUT /v1/projects/<project>/language  {"language":"German"}   (admin)
# at creation: POST /v1/projects  {"id":"…","name":"…","question_language":"German"}
```

Surfaced as a **`language_hint`** on `takomo_next` / `takomo_claim` /
`takomo_start` / `takomo_show`, as **`question_language`** on
`takomo_workflow`, as a reminder in the `takomo_ask` result and its tool
description, and as a line in the MCP server instructions. The inbox also shows
it as a reminder to the answering human.

### Style guide

A project can also declare its **house style for the text agents write** — ticket
titles and bodies, comments, and human-facing questions. This is the place for
the taste that would otherwise live in a client-side prompt: how long, what
voice, what an agent should not do.

```sh
takomo project style takomo "Two sentences max. Plain language, no marketing voice. \
Prefer a yes/no or a short choice over an open question."

takomo project style takomo --file docs/ticket-style.md   # multi-line, from a file ('-' for stdin)
takomo project style takomo                               # show the current guide
takomo project style takomo --clear
# over HTTP:  PUT /v1/projects/<project>/style  {"style_guide":"…"}   (admin)
# at creation: POST /v1/projects  {"id":"…","name":"…","style_guide":"…"}
```

Surfaced as a **`style_hint`** on `takomo_new` / `takomo_next` /
`takomo_claim` / `takomo_start` / `takomo_show`, as **`style_guide`** on
`takomo_workflow`, and echoed on the `takomo_ask` / `POST /v1/questions`
response — `takomo_new` and `ask` carry it because those are the moments an agent
has just written something and can still fix it.

It is capped at **2000 characters** (`422 project.style_guide_too_long` over
that). The cap is the point: the guide rides along on every work-loop response,
so it has to stay a handful of conventions an agent will actually read, not a
second copy of the project's documentation. Put anything longer in your repo docs
and reference it from the guide. A blank string clears it, same as `null`.

**Why here and not in a client config.** A committed client config (`.takomo/`)
is only read by the CLI — an agent on MCP, or any other client, never sees it,
and neither does anyone reading the project on the board. A project setting
reaches every client through the same rails as the language hint, and changing it
doesn't mean redistributing a skill.

### On the board

Both settings are editable in the **Settings** sheet on `/board`, which is the
better surface for the style guide: it is a paragraph, awkward to pass through
shell quoting, and the sheet's live character counter is the only place the
2000-character cap is visible *before* you save.

The button appears only for a token carrying the **`admin`** scope, and never in a
share session — a share link cannot reach `/v1/projects` at all, so the board
stays read-only for everyone else. Saving writes only the field you actually
changed, so editing the style guide never touches the language setting (and one
failing request cannot discard the other's edit).

The sheet re-reads the project when it opens rather than trusting the board's
cached list. That matters with more than one admin: a stale base would let your
save quietly revert someone else's.

## Timeouts

Give a question a deadline and a fallback so it does not rot:

```sh
takomo ask <id> --title "Proceed if nobody objects?" \
  --kind confirm --recommend yes --expires-in 86400 --on-timeout recommended
```

`--on-timeout`:

- `recommended` — apply the agent's `--recommend` value as the answer and resume. Because this auto-traverses the ticket's human gate (as `system`), it requires a real response window: `--expires-in` must be **at least 5 minutes**, and it is **not allowed on `approve`** questions (an approval always needs a real expert). Treat it as an audited, opt-in SLA fallback, not an instant self-approval.
- `escalate` — clear the expertise tags (open the question to the whole pool) and keep it open.
- `cancel` — cancel the ticket.
- *(omitted)* — just flag the question `expired`; a human still handles it.

The expiry sweep runs alongside the lease sweeper, handling each due question in
its own transaction so one bad question never wedges the rest.

## Notifications (off unless configured)

The board badge covers the ambient case. For push, set `TAKOMO_NOTIFY` to a JSON
array of routes mapping an expertise tag to a transport + target. With nothing
set, no notifications fire and the deploy stays secret-free.

```json
[
  { "expertise": "domain:billing", "transport": "slack",   "target": "https://hooks.slack.com/services/…" },
  { "expertise": "domain:legal",   "transport": "email",   "target": "legal@acme.example" },
  { "expertise": "*",              "transport": "webhook", "target": "https://ops.acme.example/takomo" }
]
```

A question matches a route when the route's `expertise` is `"*"` or is one of
the question's tags (a question with no tags matches only `"*"` routes).

- **slack** — POSTs `{ "text": … }` to a Slack incoming-webhook URL.
- **webhook** — POSTs the full question JSON (plus a rendered `text`) to any URL. This is the escape hatch for Discord, PagerDuty, or a transactional-email HTTP API.
- **email** — sends via SMTP. Set `TAKOMO_SMTP_URL` (e.g. `smtps://user:pass@smtp.example.com:465`) and `TAKOMO_SMTP_FROM`.

Set `TAKOMO_PUBLIC_URL` so notifications link straight to your `/board`.

Dispatch is fire-and-forget: it never blocks or fails the ask, and failures are
logged to stderr.
