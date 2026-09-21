# Copilot across Takomo — discussion brief

Recorded 2026-09-21 from the brainstorm started on 2026-09-17.
Status: open discussion, not an approved design or implementation plan.

## What we wanted to explore

Extend the Codex App Server integration used in document discussions into a
copilot available throughout Takomo: documents, board, tickets, mindmaps and
verification. Keep it easy to reach while people navigate and work, with enough
context to discuss what they are looking at.

The original questions were where to put it, how large it should be, which
interaction patterns fit, whether it needs screen awareness, whether it should
float or stay anchored, and whether several discussions should coexist.
We did not settle these questions. The following suggestions were starting
points from the assistant, awaiting product discussion.

## Placement, size and presence

| Option | Benefit | Tradeoff |
| --- | --- | --- |
| Resizable right dock | Stable place to discuss work alongside its source | Reduces the working canvas and competes with detail/review panels |
| Floating conversation | Preserves the underlying page width; convenient for brief questions | Covers content and canvas controls; long discussions feel cramped |
| Full conversation workspace | Room for planning, comparison, sources and conversation history | Takes attention away from the original page |
| Collapsed global control | Available throughout the app with little visual cost | Less prominent; needs clear ready/waiting indicators |

Starting suggestion: a labeled Copilot control in the global header, opening a
right dock around **420 px**, resizable roughly **360–560 px**. Provide an expand
action for focused work and remember the user's open/closed state. These are
prototype dimensions to test, not established requirements.

Below the app's 768 px desktop breakpoint, use a full-screen conversation.
On intermediate widths, consider an overlay if the dock leaves too little useful
canvas. Restore page position and keyboard focus when returning. Decide how the
dock coexists with ticket details and document review before adding another panel.

Open question: should Copilot initially be collapsed, normally open beside the
work, or become the main entry point into a conversation-led workspace?

## Context and screen awareness

Starting suggestion: make it aware of the **meaning of the current view**:
project, route, selected ticket or document section, selected text, board filters,
and explicitly pinned sources. Show removable context chips before sending and
record the source IDs and versions with the message.

Screen awareness does not inherently require screenshots. Structured app context
is useful for questions such as “what is blocking these tickets?” Screenshot
attachments could help with layout, diagram geometry or “why does this look
wrong?” Prefer an explicit attachment action with preview for that case.

Navigation needs predictable rules:

- Capture the context of a sent request; navigating must not silently retarget
  work that is already running.
- A follow-page mode could update the next message's context visibly.
- A pinned mode could keep the original topic while the user explores elsewhere.
- Switching projects must preserve project boundaries and make conversation scope
  clear. Permission checks apply to source retrieval and actions.

Open questions: what context is automatic, what requires selection, when does a
conversation follow navigation, and how do we distinguish sources it can access
from sources it actually read?

## One or several discussions

Starting suggestion: support **several saved conversations, one visible at a
time**, with titles, source links, drafts, unread markers and queued/running state.
Group them by intention: “Release readiness,” “Retry behavior,” or “Review this
section.” Navigation alone should not create a new conversation.

A contextual “Ask Copilot about this” action should open the same global surface.
When the topic changes, let the user choose the current conversation or a new one.
Keep existing document and section histories reachable without silently merging
them. Split chats or multiple windows could follow if comparison becomes a real
need.

Separate the ability to maintain several conversations from simultaneous model
execution. Queueing, bounded concurrency, and conflicting proposals need their
own decisions. Each transcript should have clear ordering and status.

Open questions: project-shared versus private conversations; naming and archiving;
when to branch; and how much parallel work people actually need.

## Interaction patterns

- Open from the global control, a contextual action, or a shortcut chosen after
  checking existing bindings.
- Preserve drafts and transcript position when closing or switching conversations.
- Start with discussion; offer drafting or actions where useful. Document edits
  remain proposals that humans accept or reject through the existing review flow.
- Link answers to their sources, and expose captured versions when content changes.
- Distinguish queued, running, ready, waiting for input, failed and stopped states.
  Show useful progress without reopening the panel or stealing focus.
- Make Stop explicit. Distinguish guidance added to an active turn from a message
  queued for the next turn.
- When background work finishes, badge the global control and restore the correct
  conversation when opened. Explain failures and the effect of retrying.

## Technical basis and limits of the exploration

The September 17 inspection used the richer integration in the local
`feat/codex-connection` worktree at `a557bef`; the shared checkout was older.
Its document documentation described persistent project-shared discussions,
selected context, pins, captured source versions and resumed Codex threads.
Its worker documentation described sequential job claims, restricted tools,
disabled images and completion-based replies. These observations were not a
verification of the deployed application and must be checked against the chosen
implementation baseline before estimating work.

The [Codex App Server documentation](https://learn.chatgpt.com/docs/app-server)
consulted during the brainstorm described thread creation/resumption/forking,
streamed events, steering and interruption. These are protocol building blocks;
they do not establish which capabilities Takomo currently exposes.

Extending the integration would require a global conversation model, context
adapters for additional pages, appropriate event delivery and permission-aware
source/action access. The UI placement should not dictate execution concurrency.

## Where to resume the discussion

1. Pick the primary use case: quick contextual help, sustained thinking alongside
   work, or directing work through conversation.
2. Choose default visibility and compare dock versus float on real board,
   document and mindmap layouts.
3. Agree on follow-page versus pinned context behavior and project switching.
4. Decide conversation ownership, sharing and the need for parallel discussions.
5. Define the initial boundary between discussion, proposals and executed actions.

A possible first experiment is a global dock that resumes document discussions,
followed by a read-only board discussion. Check whether people understand the
active context, can resume after navigation, and still have enough working space.
This experiment has not been approved for implementation.

The local interactive exploration is preserved at
`.lavish/copilot-brainstorm/index.html`, using Takomo's Aquarelle tokens. Its
messages are illustrative; it does not call Codex. The review session and its
temporary tunnels were shut down at the user's request. No product direction was
selected through that review.
