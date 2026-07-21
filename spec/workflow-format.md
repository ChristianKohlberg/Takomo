# Workflow definition format

A workflow is the per-project state machine. It is data, not code: a YAML (or JSON) document validated on upload (`PUT /v1/projects/{p}/workflow`) and enforced by the server on every `POST /tickets/{id}/transition`. There is no other way to change a ticket's state.

## Format

```yaml
name: factory-default
initial: brief

states:
  - id: brief          # inbox: anything, zero quality bar
    category: todo
  - id: spec           # spec agent is developing it
    category: in_progress
    claimable: true    # spec work is claimed like any other work
  - id: needs-decision # a human answer is required to proceed
    category: blocked
  - id: ready          # specced, approved, unblocked → the dispatch queue
    category: todo
    claimable: true
  - id: implementing
    category: in_progress
  - id: review         # PR open, evidence attached
    category: review
  - id: done
    category: done
    terminal: true
  - id: cancelled
    category: cancelled
    terminal: true

transitions:
  - { from: brief,          to: spec }
  - { from: brief,          to: cancelled }
  - { from: spec,           to: needs-decision }
  - { from: needs-decision, to: spec,           requires: [scope:human] }
  - { from: spec,           to: ready,          requires: [scope:human] }   # spec approval gate
  - { from: spec,           to: cancelled }
  - { from: ready,          to: implementing,   requires: [claim] }
  - { from: implementing,   to: needs-decision }
  - { from: implementing,   to: review,         requires: [claim] }
  - { from: implementing,   to: ready }          # released/failed → back to queue
  - { from: needs-decision, to: implementing,   requires: [scope:human] }
  - { from: review,         to: implementing }   # review feedback → back to work
  - { from: review,         to: done,           requires: [scope:human, guard:no_open_children] }
  - { from: review,         to: cancelled,      requires: [scope:human] }
  - { from: ready,          to: cancelled }

guards:
  no_open_children:
    description: every child ticket must be in a terminal state
```

## Semantics

**States.**
- `id` is free-form per project; `category` is one of the six fixed categories (`todo`, `in_progress`, `blocked`, `review`, `done`, `cancelled`) so generic tooling (boards, metrics, the ready queue) can reason about any project without knowing its state names.
- `claimable: true` marks states whose tickets enter the ready queue (when unclaimed and unblocked). The queue is therefore workflow-driven; there is no separate "ready flag" on tickets.
- `terminal: true` states end the lifecycle; terminal tickets are excluded from blocking computations (a done blocker no longer blocks).

**Transitions.**
- Only listed `(from, to)` pairs are legal. Everything else 409s with `allowed_transitions` and a `remedy`.
- `requires` entries (all must hold):
  - `claim` — the caller must hold the active lease on the ticket and echo its `fence`.
  - `scope:<scope>` — the caller's token must carry the scope (`human` is the conventional gate scope; see auth.md).
  - `guard:<id>` — the named server-side guard must pass. v1 guards: `no_open_children` (all children terminal), `no_open_blockers` (all blocked_by terminal). Guard failures are 409s that name the offending ticket ids.
- **Check ordering: legality → scope → claim/fence.** The server rejects on the *first* real blocker in that order, so the headline `code`/`message` never misdirects: an undefined `(from, to)` is `transition.illegal`; a missing scope is `transition.scope` (403); only then are claim ownership and fence echoing checked. A lease holder attempting a human gate it lacks the scope for therefore sees the 403, not a fencing complaint. `allowed_transitions` is correct in every branch.
- **Human approval overrides a held claim.** A transition whose `requires` includes `scope:human`, performed by a caller holding the `human` scope, succeeds even while *another* actor holds the claim, and auto-releases that claim as a side effect (a `released` event is emitted alongside the `transitioned` event, both in one transaction). Human approval is authoritative: a human reviewer never has to wait for a worker's lease to expire to approve a gate. This override is scoped to human-required transitions only — ordinary `claim`-required transitions keep the holder lock (a non-holder is still refused with `claim.held`).
- Transitions into a `done`/`cancelled` category state auto-release any claim. Transition out of a claimable state does not release the claim (implementing keeps the lease).

**Blocking.** A ticket is *blocked* if any `blocked_by` edge points to a non-terminal ticket, or any ancestor is blocked. Blocked tickets never appear in the ready queue regardless of state. `blocked` as a *category* is for states that represent waiting-on-a-human (`needs-decision`); graph blocking is computed, not stored.

**Risk classes and yolo.** Approval-style `requires` (`scope:human`) encode the mandatory gates. A project that wants routine work to self-land defines a second transition for the same edge with narrower conditions — e.g. `review → done requires: [scope:autoland, guard:no_open_children]` alongside the human one — and mints the `autoland` scope only to the orchestrator allowed to use it for low-risk labels. Policy about *when* the orchestrator uses that power lives in the orchestrator, not the store; the store only enforces who *can*.

**Validation on upload.** A workflow is rejected (422) if: `initial` is missing from `states`; any transition references an unknown state; any non-terminal state has no path to a terminal state; or existing tickets sit in states the new workflow no longer defines (the error lists them; migrate tickets first).

**Evolution.** Adding states/transitions is always safe. Removing requires migrating existing tickets off the removed state first. The server never mutates ticket states on workflow change.

## Default workflow

The server ships exactly one built-in workflow — the `factory-default` above. Projects that want beans-style simplicity define their own five-state machine; the format is the product, the default is a suggestion.
