# Bugs and explicit codebase research

`/bugs` is a project-scoped view of ordinary `type: bug` tickets. Reporting a bug
creates one ticket, with the same ID on Bugs and Board. It does not start an agent.
Existing bug tickets need no migration. Ticket workflow, triage disposition,
severity and research status are independent.

Reports start with unknown severity, normal scheduling priority, no assignee and
Needs triage. Record observed behavior in the description; expected behavior,
reproduction steps, affected version and supporting links help investigation.
Unknown details do not prevent reporting. Confirmed/needs-information/duplicate/
not-a-bug decisions record a reviewer and rationale without closing the ticket.

## Agent controls

The CLI returns server JSON for bug commands. Stable request IDs let callers retry
an unconfirmed submission without duplicating a report, run or steering message.
A **new** ID means a deliberate new request. Research is never a side effect of
intake, reads, triage updates or process recovery.

```sh
takomo bug new 'Incorrect receipt total' --project retail \
  --body 'Expected 12, observed 13 after applying the discount.' --request-id report-123
takomo bug ls --project retail --triage needs_triage
takomo bug show retail-xxxx
takomo bug research retail-xxxx --request-id investigation-123 --message 'Check rounding'
takomo bug runs retail-xxxx
takomo bug run aj-xxxx
takomo bug steer aj-xxxx --request-id steering-123 --message 'Only affects split payments'
takomo bug cancel aj-xxxx
takomo bug retry retail-xxxx --request-id investigation-124
takomo bug set retail-xxxx --triage confirmed --severity high --note 'Confirmed from the supplied receipt'
```

`retry` takes a ticket ID and explicitly requests a new run; `cancel` and `steer`
take a job ID. Cancelling retains the ticket and recorded evidence. Research
completion makes findings ready for review, not a resolved ticket or verified fix.

Hosted and stdio MCP expose `takomo_bugs`, `takomo_bug`, `takomo_bug_update`,
`takomo_bug_research`, `takomo_bug_runs`, `takomo_bug_run`, `takomo_bug_steer`,
`takomo_bug_cancel` and `takomo_bug_research_config`. Use existing `takomo_new`
with `type: bug` to report, and existing ticket operations for workflow and
assignment. MCP and REST call the same store methods. UI actions use those REST
operations and do not require browser automation for agents.

## Local worker setup

Research extends the existing durable agent queue and local Codex App Server
worker. Follow [the agent service setup](../services/agent/README.md) for its
separate authenticated Codex home and project-scoped `agent:run` token. Existing
section conversations retain their text-only restrictions.

The operator maps repository keys to absolute local checkout paths with
`TAKOMO_AGENT_REPOSITORIES`. An admin configures which key/revision a project uses:

```sh
takomo bug config --project retail --repository retail-code --revision main --enabled true
```

Read configuration with `takomo bug config --project retail`. Ordinary write
credentials may request research, but cannot configure arbitrary filesystem
access. Repository mappings belong to the worker, not report input. No new
public Codex endpoint or shared authentication is needed: the existing worker
uses `codex app-server --stdio` locally.

Each run resolves its configured Git reference to an exact commit and records
that revision with its findings. Dynamic tools list files, search literal text,
and read numbered source lines from Git objects. Dirty working files, symlinks,
repository scripts, shell execution and network access are excluded. A single
lead researcher is used; helper agents are optional future capacity, not required
for a useful investigation. Research has a fifteen-minute deadline, bounded
source reads and response size. Project concurrency is limited at claim time;
excess work remains queued within the bounded queue.

Findings distinguish observations, hypotheses and missing runtime evidence.
This implementation performs source research, **not runtime reproduction**.
Repository access failure or Codex failure leaves the bug intact and records the
failure. Retry is explicit; transport retry of result delivery does not rerun
the model. A stopped worker loses its lease and its work is not silently restarted.

## Validation and rollout

Run focused HTTP/MCP bug tests, CLI transport tests
(`python3 clients/cli/test-bugs.py`), worker protocol tests and frontend tests.
The normal project gates still apply before integration. A browser smoke should
cover report → research → steering → cancellation → explicit retry, and confirm
that the Board has the same ticket ID.

Back up SQLite using the supported database backup mechanism before upgrading.
The migration preserves existing section conversations and adds ticket-anchored
conversations and bug metadata. Disable project research before stopping a
worker. Do not run an older binary against a database containing ticket-anchored
conversations: rollback requires the matching pre-upgrade backup and binary,
with any newer work exported/preserved before restoration. This change does not
deploy the service or alter production credentials.
