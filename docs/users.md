# Users — who work is waiting on

Takomo knew tokens, not people. Every attribution column — a ticket's
`created_by`, an entry's `author`, a question's `answered_by`, a case's
`human_by` — held the free-form `actor` string off whichever credential made the
call, and `GET /v1/whoami` echoed it back. Nothing could say *who* a piece of
work was waiting on.

It showed up first on the ask-a-human board. Routing there is **capability-only**:
a question carries `expertise` tags like `domain:billing`, and a token holding
the matching `expert:domain:billing` scope sees it under `mine` and can answer an
`approve`. That is the right gate when what you need is *a* qualified person. It
is the wrong one when you need **Ada**.

A **user** is a person: one row, global to the server, that work can be addressed
to.

## The one sentence

> **A user says who work is waiting on. A scope says what a credential may do.**

There is no login here, and a user is not a credential. Takomo has four
independent authentication paths (`tk_` bearer, `tks_` share, `tka_` answer
grant, plus the OAuth route group in front of `/mcp`) and adding a person to the
directory grants nothing at all — they cannot read a ticket, and nothing about
their row lets anybody authenticate. Every existing gate is still a scope check.

That boundary is what keeps this small. A directory that also authorized would be
a fifth credential type, and `src/auth.rs` would need a branch for it.

## The one place it bends, and what holds it

**A named assignee may answer an `approve` question.** That is deliberate: an
approval addressed to Ada should be answerable *by Ada*, without an operator
first minting her a domain-expert scope she does not otherwise need.

So an `approve` is answerable on either of two proofs, and `may_approve` in
`src/store/questions.rs` is the only place that decides:

1. a matching **`expert:<tag>` scope** — the original rule, and
2. **being the named assignee**, proven by `tokens.user`.

Which makes binding a credential to a person an *authorization* act. Four things
hold that line, and each exists because the obvious alternative is exploitable:

- **Binding is admin-only, and stated, never inferred.** `actor` is a free-form
  string any `write` token can put anything in, so "the actor looks like Ada" can
  never mean Ada. `takomo token create --user ada` is the act, and it is
  auditable.
- **Identity is not a scope string.** Scopes are free-form — `expert:<tag>`
  proves it — so a `user:usr-abc` scope would be a *forgeable* identity, and an
  admin minting one would be handing out Ada's approval authority. Identity rides
  on `AuthCtx.user` and is passed explicitly as `Answerer`, whose whole reason to
  exist is that the parameter beside it (`resume_to`) has the same type.
  `a_user_scope_string_cannot_forge_assignee_identity` in `tests/api.rs` is the
  guard.
- **Relaying an `approve` stays refused.** A relay records a decision made out of
  band, so the name in it is a claim the relayer is making rather than one the
  server can vouch for — including when the relaying credential is itself bound
  to that person. The channel is the problem, not the identity.
- **An answer link for a person-gated approval is only mintable by them.**
  Whoever holds the link string can spend it, so a colleague minting "Ada's" link
  would be approving as Ada. `admin` is not a way round it either: in this
  codebase admin has never stood in for the authority a gate names. The admin act
  that reaches a person with no credential is binding them one.

Disabling someone withdraws exactly this: they stop being assignable, and the
assignee route to approving closes with it. It does **not** revoke their tokens —
what a credential may do is its scopes. Revoke the token to end access.

## Where a person shows up

**Addressed to, on the questions board.** `assignee` on `POST /v1/questions`, or
`POST /v1/questions/{id}/assign` afterwards — which is the usual case, because
the agent that raises a question knows a billing question when it writes one, not
which colleague owns billing this month.

Assignment is **routing, not a lock**. Any `human` token may still answer the
three ordinary kinds, so a decision is never stranded because the assignee is
away. What it changes is where the inbox sorts it, and what `mine` means:
`?mine=true` is now the union of *addressed to me* and *my expertise*, because a
person owes an answer on either. `?assignee=<handle>` reads one person's queue,
and `?assignee=none` is the triage read — what nobody has been asked yet.

**Named, in a collection.** `/initiatives` already spoke `person:ada`: on an
initiative's tags, and as an entry's `source`. Those references now resolve
against the directory, so a document shows "Ada Lovelace" where it used to show a
slug — and an unknown handle still degrades to the slug exactly as before.
Nothing about initiatives changed shape; the same string means more.

**Verification, later.** The checklist's checks and cases record who gave a
verdict as an actor string, and a `human` verdict is already the line between "an
agent observed this" and "a person approved it". Assigning a check to a person
drops onto this directory when that surface arrives; nothing is built for it yet.

## Why global, with membership

A person is not per-project, so the row is global — like `tokens`, and unlike the
project tag registry. That is what makes `answered_by: ada` resolve to the same
human from anywhere, which a per-project row could not.

`user_projects` then says **who may be handed work where**. It is directory
scoping, not access control: a token's own `projects` allowlist still decides
what that credential may read or write. Because a named assignee may approve,
membership is also a second fence in front of that authority — you cannot hand a
decision to somebody who was never put on the project.

Ending a membership deliberately leaves open questions addressed to that person
alone. Silently retracting a decision would leave it with nobody looking at it;
they simply cannot be handed anything new there.

## The handle is shared with the tag registry, on purpose

A user handle is validated by the *tag* handle rule (`handle_shape_ok` in
`src/store/tags.rs`), so `person:<handle>` stays a legal tag reference. That one
decision is what makes the convention that predates the directory converge on it
instead of forking into two vocabularies for the same person.

The two stay distinct in what they *do*, and there is one command for each:
`takomo tag` attaches a `person:<handle>` **reference** to one project's tickets,
`takomo user` is the **directory** of people a decision can be addressed to.
Tagging still affects nothing — the tag registry's own header says so.

`takomo person` used to be sugar over the tag kind and is retired, because two
people-shaped commands where one quietly did not add a person is a trap:
`takomo person add ada` wrote a registry row almost nothing reads, while reading
like it had put Ada in the directory. It now prints where to go instead, and
`takomo person ls` runs the directory listing, which is what somebody typing it
almost always meant.

Where the two meet, the directory wins on identity: a `person:` tag whose handle
names somebody in the directory carries them as `person` on the wire, so a reader
sees "Ada Lovelace" rather than the stub label lazy-creation wrote (which is the
handle again), and `?q=` searches that name. The tag row's own `label` is left
alone — resolving is not copying, so a rename in the directory is immediately
right everywhere instead of drifting.

## There is no delete

`disabled_at` is a gate, in the same idiom as `projects.archived_at`: reversible,
and it changes nothing else about the person. Deleting the row would make an
answered question's `answered_by` unreadable after the fact, which is the one
thing an audit trail may not do.

## Commands

```sh
takomo user new ada --name "Ada Lovelace" --project demo
takomo user ls --project demo          # who can be handed work here
takomo user member ada other-project
takomo user disable ada                # stop new work reaching her; keep the record
takomo tag ls --kind person            # the references, and who is in the directory
takomo token create --actor ada --scopes read,write,human --user ada

takomo ask <ticket> --title "Which key?" --assignee ada
takomo assign <qid> ada                # or: takomo assign <qid> --none
takomo questions --mine                # addressed to me, or in my domain
takomo questions --unassigned          # what nobody has been asked yet
```

`takomo user` also exists as a `takomo` binary subcommand operating on the
database directly, because the first person has to exist before any credential is
bound to them and `POST /v1/users` needs an admin token to call. Shell access is
the root of trust (`spec/auth.md`).

Over MCP: `takomo_users` (a free read) to find who, `assignee` on `takomo_ask`,
and `takomo_assign` afterwards.

## Related

- `docs/ask-a-human.md` — the board this changes most
- `spec/auth.md` — why a user is not a credential
- `spec/openapi.yaml` — `/v1/users`, and `assignee` on the question schemas
