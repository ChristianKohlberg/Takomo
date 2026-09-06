# Epics page

Open `/epics` from **Epics**, beside Board in the navigation rail. It uses the
selected project's workflow and opens the existing ticket details when an epic
is selected. **New epic** creates an epic in that workflow's initial state;
read-only viewers and archived projects cannot create one.

The page has a compact heading with a result count and creation action. One
wrapping toolbar holds title/ID search, Active/All, a Filters popover, and sorting.
State, initiative, and claim filters compose; applied filters remain visible in a
short summary and can be cleared together. Active excludes the project's terminal
workflow states. The project picker resets filters when changing projects.

The initial order is **Needs attention first**, then newest activity within each
group. Users can instead choose recent activity, title, or most complete. Attention
means tasks awaiting answers, tasks in the blocked category, a blocked epic, a
claim without movement for at least 24 hours, or a state/progress contradiction.
An empty epic is ordinary planning and does not count as an attention signal.
`awaiting_answer` counts affected tasks, not individual questions; the UI labels
that distinction. Backlogged/claimed work is not labelled blocked.

Each row gives the title the most weight, with initiative context and a secondary
ID. State, a small progress bar with completed/total counts, holder, and last
activity follow. Empty epics say **No tasks**. Claim durations and detailed task
breakdowns stay in details. Attention labels appear only when present and use
text alongside outline icons; color is not their only signal.

The roadmap's `last_activity_at` is the newest `updated_at` on the epic or its
descendants, whether claimed or unclaimed. It is an update timestamp, not a claim
of active work or time spent. Older servers without this field can still supply
claim activity; otherwise the UI explicitly shows unknown activity and sorts it
last. Data failures are shown as retryable errors, not as an empty project.

Questions attached directly to an epic appear separately from descendant tasks awaiting answers; either signal moves the epic into the attention group.
