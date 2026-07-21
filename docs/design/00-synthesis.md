# Synthesis: adopt vs. build (July 2026)

Requirements: hosted central sync store · basic/token auth · epics → tasks (→ subtasks) · free-form metadata for agent context · proper state machine · not fancy, agent-ergonomic.

Detail reports: [01-beans.md](01-beans.md) · [02-agent-native-trackers.md](02-agent-native-trackers.md) · [03-hosted-trackers.md](03-hosted-trackers.md)

## The landscape in one paragraph

The market splits into two halves that each solve half the problem. **Agent-native trackers** (beans, beads, Backlog.md, task-master solo) have exactly the right ticket semantics — epics, dependency graphs, JSON metadata, `--json` CLIs, prime prompts — but are repo-local/git-synced by design; none is a hosted, authenticated service. **Hosted trackers** (GitHub, Linear, Plane, Redmine, OpenProject) are central and authenticated, but almost none enforces a state machine (only Jira/OpenProject/Redmine do), and none has agent primitives like atomic claiming. The few projects that sit exactly in the middle (saltbo/agent-kanban, Adam-Dangerfield/Agent-Kanban, kandev) are 3–5 months old with 3–450 stars. Tegon, Vibe Kanban, Height, Kitemaker, Focalboard are dead; hmans/beans itself is in de-facto maintenance mode.

## Requirements scorecard

| Candidate | Hosted+auth | Epics/hierarchy | Metadata | State machine | Maturity | Notes |
|---|---|---|---|---|---|---|
| hmans/beans | ✗ (repo-local, zero auth) | ✓ | tags only | ✗ (5 fixed, free-form) | fading | prior art for ergonomics |
| beads (bd) | ◐ (Dolt SQL server mode, MySQL auth) | ✓ | ✓ arbitrary JSON | ✗ (4 fixed; rigor via dep graph + atomic claim) | ✓ 25k★, daily | best existing 4-of-5 |
| GitHub Issues | ✓ (SaaS, PAT/App) | ✓ 8-level sub-issues | ◐ typed issue fields, no JSON | ✗ (open/closed) | ✓✓ | 500 writes/hr/identity |
| Redmine | ✓ self-host, tiny | ✓ unlimited | ✓ custom fields | ✓ enforced (silent-ignore wart over REST) | ✓ old but alive | dated API |
| OpenProject CE | ✓ self-host | ✓ | ✓ | ✓✓ enforced, 422 + allowedValues | ✓ | heavy (~8 GB), verbose HAL |
| Linear | ✓ SaaS only | ✓ | ◐ attachment KV | ✗ | ✓ | $, closed, 250-issue free cap |
| Plane CE | ✓ self-host | ✗ (epics = Pro) | ✗ (Pro) | ✗ (Business) | ◐ | paywalled out of fit |
| saltbo/agent-kanban | ✓ (Ed25519 per-agent auth) | ✗ no epics | ◐ | ✓ enforced claim/review/complete | ✗ 4 mo, 406★ | closest young server |
| Adam-Dangerfield/Agent-Kanban | ✓ token auth, Postgres | ✓ epics→stories→tasks | ✓ | ◐ fixed w/ audit trail | ✗ 3★ personal tool | full shape, no community |
| task-master team (Hamster) | ◐ closed SaaS, OAuth only | ◐ no epics | ✓ JSON | ✗ | ◐ OSS stalled | vendor lock |

## Conclusion

**Nothing mature satisfies the spec as written.** The three defensible moves:

### Option A — GitHub Issues as the store (fastest, zero ops)
Sub-issues (8 levels) + issue types give the epic hierarchy; issue fields (GA 7/2026) + a fenced JSON block give metadata; PAT/App auth; the best first-party MCP server anywhere. The state machine must live in the client/orchestrator layer (a thin shared skill/CLI that owns legal transitions). Watch the ~500 content-writes/hr per identity limit.
**Fit: 4/5. Cost: convention instead of schema; no store-side enforcement.**

### Option B — Redmine (or OpenProject) self-hosted (store-enforced state machine)
Redmine: one small container, token auth, unlimited hierarchy via trackers (Epic/Task/Subtask), rich custom fields, real per-tracker×role transition matrix. Must wrap the REST silent-ignore wart with pre-check + read-after-write. OpenProject does the same enforcement loudly (422 + allowedValues) at ~4× the footprint and a much chattier API.
**Fit: 5/5 functionally. Cost: 2005-era ergonomics; needs a thin agent-facing wrapper/MCP anyway.**

### Option C — Build "Takomo" (validated gap, small spec)
The gap is real and the ecosystem keeps converging on it without a mature winner. The spec is small: single binary (Go or TS) + SQLite/Postgres, bearer-token auth, epics→tasks→subtasks, JSONB metadata, **configurable state machine with enforced transitions + atomic claim/lease**, REST + MCP + thin CLI, SSE for wakes. Borrow beans' ergonomics (prime prompt, exact-match body edits, --json), beads' claiming and dependency-ready queue, agent-kanban's enforced transition ops. Publishable — nothing mature occupies this slot.
**Fit: 5/5 by construction. Cost: it's a project to own; interim solution needed while building.**

### Recommendation
Pragmatic path: **start on Option A (GitHub Issues) this week** — zero setup, agents already authenticated, hierarchy + fields are good enough, and the transition-enforcement layer you write for it (a small shared client) is exactly the API contract for **Option C** if/when the build is justified. Choose B instead of A only if store-side enforcement or self-hosting is non-negotiable from day one.
