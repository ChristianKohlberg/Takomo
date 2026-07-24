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
  --recommend yes
```

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
your expertise, inline answering, withdraw, and "create answer link"). The
`/board` also has a lightweight **Ask a human** drawer with an unread badge for
answering in context. Or over the API:

```sh
takomo answer q-9f3ka2xz yes --note "confirmed with the data team"
```

Answering requires the **`human`** scope — it *is* the human authorization gate,
and it performs the ticket's human-gated resume transition (in the factory
default, `needs-decision → ready`). `POST /v1/questions/{id}/answer` / the
`takomo_answer` MCP tool are the other two surfaces.

An agent that no longer needs its answer withdraws it: `takomo withdraw <qid>`.

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

## Per-project question language

A project can declare the human-facing language its questions should be written
in — e.g. **German** for a revamp project whose reviewers are German-speaking,
even though the agents and the underlying tickets work in English. Set it once
(admin):

```sh
takomo project language <project> German      # or: takomo project create … --language German
# over HTTP:  PUT /v1/projects/<project>/language  {"language":"German"}   (admin)
# clear it:   takomo project language <project> --clear
```

The point is to reach the **agent at the source**, so the question text lands in
the right language rather than being flagged after the fact. The setting is
surfaced wherever an agent works:

- a **`language_hint`** on `takomo_next` / `takomo_claim` / `takomo_start` /
  `takomo_show` — so the agent sees it *before* it asks;
- **`question_language`** on `takomo_workflow`;
- a reminder in the `takomo_ask` result and its tool description;
- and a line in the MCP server instructions.

It's a **soft nudge**, never enforced (language can't be reliably detected
server-side). The inbox also shows it as a reminder to the answering human. It's
a project setting, so every viewer and agent sees the same thing.

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
