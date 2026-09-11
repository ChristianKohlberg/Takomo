# Per-project document classification

Settings → Projects → select a project → General → Document classification
separates scheduling from the policy for reviewing matches. Only a project-authorized
administrator can change these settings.

- **Automatic** schedules matching for new tickets and material changes to title,
  body, parent or project. Explicit requests also work.
- **Manual** schedules only explicit single-ticket or project matching requests.
  Saving this mode cancels queued automatic classification jobs and removes their
  pending scheduling entries. Explicitly requested jobs remain queued.
- **Off** rejects new classification requests, cancels all queued classification
  jobs in the project and clears pending scheduling entries.

Running classifications finish in all modes. Existing document references and
suggestions remain available for review, and manually attaching links is always
available. Other agent job types and other projects are unaffected. Saving reports
how many queued jobs were cancelled; cancellation preserves their history.

The existing Suggest / Automatically accept clear matches setting controls the
result, independently of scheduling. Suggest does not mean no model work.

Automatic remains the default for compatibility. Turning it on does not backfill
existing tickets: use the explicit project matching action if that is intended.
Manual project requests retain the requesting actor through the background sweep.
Cancelled jobs do not prevent a later explicit request for the same ticket revision.

The API adds optional `scheduling: off | manual | automatic` to
`PUT /v1/projects/{id}/document-classification-config`. Omitting it preserves the
current setting, including when older clients save the match-review policy. GET
returns the effective setting. PUT additionally reports `cancelled`.

Deploy the server/UI together. No worker update or environment variable is needed.
The database upgrade adds scheduling and pending-request attribution columns and
replaces scheduling triggers. Existing queues and match-review policies survive
upgrade and reopen. Verify the saved policy and cancellation count before enabling
any previously unauthorized worker for a large backlog.

For rollback, prefer restoring scheduling to Automatic on the new server first;
an older server does not enforce Manual/Off scheduling. Cancelled jobs remain
cancelled and are not resurrected by rollback. Preserve the database and retain
normal backups; existing references are not removed by this setting.
