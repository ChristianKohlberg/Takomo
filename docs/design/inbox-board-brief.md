# Design brief: takomo `/board` and `/inbox`

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

## Surface 2 — `/inbox` (new; the main design target)
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
CSS custom-property tokens, Jira/Trello-adjacent. Light: `--bg #f4f5f7`, `--panel #fff`,
`--text #172b4d`, `--muted #6b778c`, `--border #dfe1e6`, `--accent #0052cc`;
priority/urgency: `--crit #de350b`, `--high #ff8b00`, `--normal #0065ff`, `--low #6b778c`;
chips `--chip-bg #dfe1e6` / `--chip-text #42526e`. Dark variants exist for all. Keep this
token system; evolve layout / typography / hierarchy, not the palette identity.

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
