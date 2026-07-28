# Design brief: takomo `/board` and `/inbox`

> **Historical prompt artifact — written against an HTML snapshot that no longer exists.** This was a one-shot
> brief to seed a design project, not a description of the surfaces as they are. The redesign it asked for has
> since landed, so its "existing state" claims are the *old* state. Its framing of the problem (one system, two
> mental models; chip soup; triage ergonomics) is why the file is kept.
>
> Its **"Existing visual language" section was corrected in place** rather than annotated, because the palette
> it listed would have driven a regression — see the note there. Everywhere else the original prose stands, with
> **Shipped:** notes where the code has moved past it.
>
> If you are briefing design work today, re-derive the current state from `src/board.html` and `src/inbox.html`
> and from the **Architecture** section of [CLAUDE.md](../../CLAUDE.md); do not copy facts out of this file.

Seed this into a Claude Design project (paste as the first message / project brief),
then attach the current `src/board.html` and `src/inbox.html` as the "current state"
so it designs *forward* from what exists, not greenfield.

## Product in one line
**takomo** is a self-hosted, single-binary (Rust + SQLite) task tracker that a fleet of
**AI agents**, orchestrators, and **humans** all talk to over HTTP. These two web
surfaces are the human-facing windows into it. They are **static, dependency-free HTML**
served from the binary (vanilla JS, no build step, no external CDNs/fonts) and
**token-gated** (the viewer pastes a bearer token; every fetch carries it). They must be
**theme-aware** (light default + dark via `prefers-color-scheme`) and **never scroll
horizontally**.

## Surface 1 — `/board` (exists; refine)
A read-only **kanban**. Columns = a project's workflow states (per-project state machine,
e.g. `brief → spec → ready → implementing → review → done`, plus `blocked`/`cancelled`).
Cards = tickets: `id`, `title`, `type` (task/bug/epic/spike), `priority`
(critical/high/normal/low — drives a left-border color), `labels`, `claim` holder (an
agent), `archived`. A card opens a **detail drawer**: body, comments, dependencies
(blocked_by/blocks), links (PR/branch), state. Live updates via polling the event log.
A **share mode** (`#s=<token>`) renders a read-only, scope-bounded view. Recently gained a
lightweight **"Ask a human" drawer + unread badge**.

> **Shipped:** `/board` is no longer read-only. It answers questions from its ask-a-human drawer, and it
> PUTs per-project settings from a settings sheet (those two PUTs are the admin-gated part; the sheet itself
> opens read-only for any token). Only *share mode* is read-only. Two other details in the paragraph above
> have moved: priority no longer drives a left-border color — it is a 4-bar rank glyph on the card plus a
> colored word in the drawer — and the board carries full DE/EN string tables. Live updates poll
> `GET /v1/events?since=<cursor>`; the SSE stream is not used, because the browser `EventSource` API cannot
> set an `Authorization` header.

## Surface 2 — `/inbox` (new; the main design target)

> **Shipped:** "new" dates this section. `/inbox` is now the board's equal in weight — the two files are
> within a hundred lines of each other — and it carries DE/EN string tables, deep-linkable and bookmarkable
> `#q=<id>&folder=…&project=…&ticket=…` URLs, and follow-up threads on a question. The three-pane shape, the
> four folders and the kind-adaptive answer controls described below all landed; the "first-cut"
> characterisation at the bottom of this file no longer applies.

An **email-style triage surface** for the ask-a-human board. A stuck agent raises a
**question** tied to a ticket; a human answers, which unblocks the work. Three panes:
- **Folder rail:** status folders with counts — Open / Answered / Withdrawn / Expired. A
  **project** selector (All or one) and a **"mine"** toggle (the viewer's expertise).
- **List pane:** scannable rows — `asked_by` (which agent), `title`, `ticket` chip,
  `kind`, `urgency`, `expertise` tags, age. Sorted urgency-then-age.
- **Reading/answer pane:** the full question + an answer control that **adapts to the
  kind**, a note field, **Withdraw**, and **Create answer link** (single-use link for an
  outside expert).

### Question data (what you're designing around)
`kind`: `confirm` (yes/no) · `approve` (approve/reject, gated to a domain expert) ·
`choose` (pick one of N options) · `clarify` (free text). `mode`: `blocking` (parks +
resumes the ticket) or `advisory` (routed decision, no state change — e.g. epic-level).
Plus `title`, `body`, `options[]`, `recommended` (agent's suggested answer), `expertise[]`
(tags like `domain:billing`), `urgency`, `status`, `answer`, `answered_by`, `resolved_to`,
`expires_at`. A minimal third surface exists: the **single-question answer page**
(`#a=<token>`) an outside expert opens from a link — just the one question + answer control.

## Existing visual language (match it — the surfaces must feel like one product)

> **Corrected, not annotated.** The hex values that stood here described a Jira/Trello-adjacent palette
> that no longer appears anywhere in `src/`, and the section told the reader to "keep this token system" — so
> following it would have reverted a redesign that has since landed. The replacement below describes what is
> actually in the tree. It is deliberately not a value list: read the tokens from the code.

The mechanism is unchanged — CSS custom properties in a `:root` block, light default plus a
`prefers-color-scheme: dark` override, every value present in both themes. The palette is the
self-described **Aquarelle** one, shared by both surfaces: a cool paper-grey ground, white panels, a
deep-blue `--accent` for primary and critical emphasis, a warm earth `--crit` for error/reject/bug, a
green `--ok`, and a blue urgency ramp descending to grey (`--high` → `--normal` → `--low`).

The **authority is the `:root` block at the top of `src/board.html`**, mirrored in `src/inbox.html` (the
inbox carries a few extra tokens of its own). Read it there; do not copy values out of this file. The
still-valid instruction is the one this section ended on: evolve layout, typography and hierarchy, not the
palette identity.

Also load-bearing, and stated in the code's own comment: **each attribute is encoded exactly once.**
Priority is a rank glyph on a card and a single colored word in the drawer, identifiers are monospace, and
the card that matters is a tinted field — deliberately *not* a colored left border (see the note under
Surface 1).

## The design challenges (what "good" must solve)
1. **One system, two mental models.** Board is *spatial* (kanban); inbox is *sequential*
   (triage). Shared header, chip language, type scale, density — but each optimizes its own
   interaction.
2. **Chip soup risk.** A question carries urgency + kind + mode + expertise + ticket; a card
   carries priority + type + labels + claim. Strict hierarchy so the *one thing that matters*
   dominates and metadata recedes.
3. **Triage ergonomics.** Email-grade scannability: a heavy queue stays calm, "open" feels
   unread/actionable, and a human answers **without leaving the list** (keyboard flow, obvious
   primary action). The reading pane must instantly convey *what am I deciding, what does the
   agent recommend, and what happens when I answer* (resumes a ticket vs. advisory-only).
4. **Kind-adaptive answer controls.** One reading pane, four answer shapes: binary
   (confirm/approve), option-set (choose), free text (clarify). `approve` should read as a
   heavier, expert-gated gate than `confirm`.
5. **Scale + empty states.** Fleets produce bursts; design for a long queue *and* a
   satisfying empty state. Counts and urgency ordering carry the load.
6. **Non-disruptive live updates.** Content refreshes on a poll; never wipe a half-typed
   answer, lose the selected item, or jump scroll.
7. **Attribution & block-and-resume.** Make visible: which *agent* asked, which
   *human/expertise* should answer, and that answering **unblocks a ticket** (show
   `resolved_to`). Trust/auditability are features.
8. **Cross-surface navigation.** Jump from an inbox question to its ticket on the board and
   back without losing place.
9. **Responsive collapse.** 3-pane → single column (list → detail) on narrow screens; board
   columns scroll within their own container, page never does.
10. **First-run = a token gate.** Entry is "paste a token"; low-friction and reassuring
    (what scope you need, what you'll see).

## Suggested Claude Design project structure
- **North-star exploration (2–3 directions):** *calm-productivity* (whitespace, restrained
  color, one accent) · *dense-operator-console* (compact rows, more signal) · *focused-triage*
  (big reading pane, minimal list). Judge against challenge #3.
- **Shared component inventory:** header/token-gate, chip system (priority vs urgency vs tag
  vs status as visually distinct roles), card, list-row, folder-rail item + count, reading
  pane, the four answer controls, empty states, live/refresh indicator — dark + light each.
- **Screens:** board (columns + card + detail drawer), inbox (3-pane: open queue, an
  answered item, an empty folder), the outside-expert single-question answer page, and the
  responsive/mobile collapse of each.
- **Deliver:** the token set (reuse/extend existing vars), component specs, annotated
  mockups — noting that final output is **hand-written vanilla HTML/CSS/JS, no external
  assets**, so keep everything expressible without a framework or CDN.

## Current state to attach
- `src/board.html` — the existing kanban board (token gate, poll loop, detail drawer,
  share mode, ask-a-human drawer). Ground truth for the token palette and JS patterns.
- `src/inbox.html` — the first-cut 3-pane inbox (folder rail, list, reading/answer pane).
  The primary thing to elevate.
