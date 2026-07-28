# Changelog

All notable changes to takomo are documented here. The format is loosely
based on [Keep a Changelog](https://keepachangelog.com/), and the project aims
to follow [Semantic Versioning](https://semver.org/). The `/v1` HTTP API evolves
additively only.

## [Unreleased]

Naming things, and proving them. Tickets can carry **tags** from a per-project
registry, so a project names the people, components and teams it cares about
instead of encoding them in free text. `done` can be made **checkable** — a
project can require the closing commit as a link. Projects can declare the
**house style** agents write in — and how long an **answer link** handed to an
outside expert stays valid — and both web surfaces can be narrowed to a single
ticket.

### Added

- **Per-project tag registry.** A project names entities of any `kind`
  (`person`, `component`, `team`, …) and attaches them to tickets by
  `kind:handle`. A new kind is just a new string — no schema change — and
  per-kind attributes live in a free-form `meta`. Tagging is reference metadata
  only: it never touches ticket state, claims, or question routing.
  `/v1/projects/{project}/tags` CRUD, `tags`/`tags_add`/`tags_remove` on ticket
  create and patch, `?tag=` / `?tag_kind=` list filters, `takomo tag …` /
  `takomo person …`, and `takomo_tag` over MCP. Tagging a ticket with an unknown
  handle lazily registers it; deleting a tag keeps ticket references and reports
  how many still point at it. Both `/board` and `/inbox` gain a two-step
  kind-then-value tag filter.

- **`guard:has_link:<key>` — prove "done" instead of claiming it.** A
  parameterized guard family: the ticket must carry a non-empty `links.<key>`.
  `has_link:commit` is the intended use — a full SHA stays checkable long after
  everyone has forgotten the ticket, and release/deploy questions derive from it
  (`git tag --contains`, `git merge-base --is-ancestor`) with no extra
  bookkeeping — but the key is free-form, so a project can demand `pr`, `run` or
  `env`. Opt-in per project; `factory-default` is unchanged, so no existing
  workflow starts rejecting anything. Rejections name the missing key and the
  remedy. `takomo link ID --commit SHA` warns on a short SHA, and `/board` shows
  a quiet `⌗ <sha>` badge — deliberately softer than the promotion badge, since
  it means "verifiable", not "shipped".

- **Per-project style guide for agent-written text.** The house style for ticket
  titles and bodies, comments and questions, declared once on the project so it
  reaches every client instead of one checkout. `PUT /v1/projects/{project}/style`
  (admin), or `style_guide` at creation; capped at 2000 characters
  (`422 project.style_guide_too_long`). Surfaced as `style_hint`, alongside
  `language_hint`, on the work loop of **every** client — `POST /v1/tickets`,
  `GET /v1/tickets/{id}`, `POST /v1/tickets/{id}/claim` and `POST /v1/ready/claim`
  over REST, the matching MCP tools, and printed by `takomo new` / `show` /
  `claim` / `next` / `start` — so an agent driving the store through the CLI reads
  the house style at the moment it has just written something and can still fix
  it. A project that sets no conventions gets no extra keys, and the hints stay
  off list responses, which are per ticket where the conventions are per project.
  Also on `takomo_workflow` as `style_guide`. Advisory, never enforced. Editable,
  with the question language, from a project-settings sheet on `/board`.

- **Answer links last a week, and a project can set its own.** The link you hand
  an outside expert to answer exactly one question now lives **7 days** by
  default instead of 3 — long enough to survive a weekend, since the expert
  answers on their own schedule and chasing them again costs more than the
  exposure of one single-use, one-question grant. Projects can set their own
  default: `PUT /v1/projects/{project}/answer-link-ttl` (admin),
  `answer_link_ttl_seconds` at project creation, `takomo project answer-ttl demo
  14d`, or a lifetime picker on the `/board` Settings sheet beside the
  question-language and style-guide fields. Precedence is most-specific-first: an
  explicit `--ttl` on the mint call, then the project default, then the built-in
  week — and the mint response now reports both the lifetime applied
  (`ttl_seconds`) and where it came from (`ttl_source`). Bounded at 30 days, the
  same cap share links carry and exactly the bound an explicit `--ttl` already
  had: an answer link is a credential handed outside the org, so no setting can
  turn it into a standing one. Unlike the other two project settings this one is
  enforced rather than advisory, so writing it needs `admin` — a non-admin sees it
  read-only on the sheet, like the rest. `takomo answer-link --ttl` and `takomo
  project answer-ttl` take the same `7d` / `24h` / `90m` / `3600s` durations
  `takomo share --ttl` does.

- **Filter the queue and the board by one ticket.** Both surfaces could already
  be narrowed by epic or expertise, but neither answered "what is still open on
  this one ticket". `/board` gets a ticket picker that composes with the epic
  filter and keeps subtasks visible; `/inbox` gets one built from the tickets
  that actually carry questions, deep-linkable as `#ticket=<id>`. A filtered
  empty queue says "no questions for this ticket" rather than "all clear", which
  would lie about the whole queue.

- **Real markdown in the inbox**, clamped long replies, and agents can revise a
  question's options (`takomo options <qid>`) after research shows the original
  set was wrong — instead of withdrawing and throwing the thread away.

- **Deep-linkable inbox**, an answer-link modal, and epic grouping on the board.

- **backlot 0.7 integration** (`backlot.yml`) — a warm, seeded instance in one
  command, with role-mapped tokens via `scripts/backlot-token.sh`.

### Fixed

- **The `/board` header wrapped onto a second row at every desktop width, and a
  third at 761px.** Measured at 1440, 1280 and 1024 it was two rows and 83.6px,
  and at 761 three rows and 129.6px — identically in English and German. It did
  not read as a two-row header anyone had designed: the first row was packed
  edge to edge and the second carried *Refresh* and *Sign out* alone on an
  otherwise empty strip, under a board whose whole job is showing columns of
  cards.

  The `Filters ▾` disclosure that shipped for phones now works at every width.
  Closed, the header is **one row and 58px at 1440 and 1280**; open, the filter
  controls reappear inline in the order and the row they always had, giving
  back exactly the two rows and 83.6px the header has today. A phone still gets
  the stacked panel — that layout needs one, a desktop does not — so this is a
  reveal, not a second layout, and it adds no new UI and no new strings.

  **The project picker deliberately stays outside the disclosure.** It is the
  only place the board says *which project you are looking at*, which is context
  rather than a filter, and context should not need a click. It costs 179px,
  which is why 1024px keeps its second row: a chosen consequence, not something
  left to fix. Hiding the picker would buy that row back and was the option
  weighed and turned down.

  Shorter labels or desktop glyphs were measured and rejected instead: they fix
  English and leave German at two rows, and German is precisely the case this
  was about. `/board` renders pixel-for-pixel identically at 390px — 0 of
  329,160 pixels differ, in both locales, with the disclosure open and closed —
  and every phone touch target still clears 44×44.

- **At phone width `/inbox` dropped its filter rail, so three of its four
  folders and the tag filter were not merely awkward but absent.** The one-pane
  phone layout collapsed `main` to a single column and set `.rail { display:
  none }`. The rail is where the folders (open / answered / withdrawn /
  expired) and the tag filter live, so a phone reader was locked to the default
  *open* folder with no way to reach an answered, withdrawn or expired question
  and no way to filter by tag at all. Everything else at that breakpoint
  degrades gracefully — the panes collapse to one, a back button appears,
  popovers go full width; this one silently removed function.

  The rail's two jobs are now relocated, and deliberately not by the same
  mechanism. **Folders become a chip strip** above the list, always on screen:
  they are navigation rather than a filter, and their counts are the inbox's
  headline signal, which should not sit behind a disclosure. The strip scrolls
  sideways, so it takes a fifth folder — or a tenth — without any layout
  assuming a number, and the active chip is scrolled into view when the folder
  or the locale changes (German chips run half again as wide as English ones).
  **The tag filter becomes a disclosure**, matching `/board`'s `Filters ▾` and
  reusing its `body.filters-open` idiom: its option set is unbounded and
  two-stage, so it cannot be a strip, and it costs no vertical space until it
  is opened. Its control is pinned beside the scroller rather than inside it,
  so an active filter is always visible and a short list always shows its
  cause. Chips and both selects are at least 44px on the short side, matching
  the touch minimum the rest of the phone layout already keeps.

  The desktop rail is untouched, and so is the `body.reading` one-pane
  mechanism: the bar is `display: none` outside the phone breakpoint, which
  keeps it out of the grid and out of the tab order, and it hides again while a
  question is being read. `/inbox` renders pixel-for-pixel identically at 1440,
  1200, 900 and 761 (0 of 1,296,000 pixels differ at 1440). No new UI strings,
  so both locale tables are unchanged.

- **The Settings sheet was invisible to a non-admin token, with no hint it
  existed.** `/board` hid `#settings-btn` outright unless the token carried
  `admin` — the one place in either SPA where a control disappeared on a scope
  rather than explaining itself. Every other `hidden` toggle in the two files is
  content-driven. A `human` token, which is what a person pasting into `/board`
  normally holds, saw no button, no disabled control and no explanation, so
  anyone who knew the feature existed concluded it was broken.

  Hiding it also cost more than it protected. Only the two writes are
  admin-gated; `GET /v1/projects` is `read` scope and already returns
  `question_language` and `style_guide`, and both values are pushed to every
  worker as `style_hint`/`language_hint` on the work loop. So the sheet was
  hiding values its viewer was already entitled to — and being handed anyway.

  The sheet now opens for any session token and goes **read-only** without
  `admin`: the values are shown, the fields are `readonly` (not `disabled`, so
  they stay focusable, selectable and copyable — reading them is the point),
  the character counter is dropped since there is no cap to hit, *Cancel*
  becomes *Close*, and Save is marked `aria-disabled` with a visible reason
  naming the missing `'admin'` scope, wired up as its accessible description.
  Following `/inbox`'s precedent, Save is deliberately not natively `disabled`:
  that drops a control out of the tab order and explains nothing, which is the
  same dead-control trap that produced this report. Pressing it re-states the
  reason instead of spending a request the server would refuse.

  The write boundary is unchanged — `PUT …/style` and `PUT …/language` still
  require `admin`, which is what a future per-project answer-link expiry will
  want. Share sessions keep no button at all, and that is not the same bug: a
  `tks_` token is confined to `/v1/shares/self*` and genuinely cannot load the
  sheet. `/board` only; the desktop sheet renders pixel-for-pixel as before.

- **A phone could open the ticket drawer and not get back out.** `/board`'s two
  drawers — the ticket detail and the ask-a-human queue — are full-bleed below
  480/520px, which is right for reading. But `#overlay`, the tap-to-close scrim,
  sits at `z-index: 20` and the drawers at `21`, so at full width the scrim was
  entirely behind them and could never be tapped. Escape closed them too, and a
  phone has no Escape key. That left exactly one exit, a 20px glyph in 2px 6px
  of padding: a measured 23.7 × 24 target, against the 44px touch minimum.

  The drawers keep their width — 390px minus the reading padding is already as
  narrow as that column should get, so uncovering the scrim by narrowing the
  sheet would have bought an exit with line length. Instead the sheet now rises
  from the bottom and stops 56px short of the top, putting the scrim back on
  screen as a full-width strip you can tap to dismiss, with the dimmed board
  showing through it. The close button becomes a real 44 × 44 target in the same
  motion, grown into the header's own padding so the header keeps its height.
  A phone now has the two independent ways out a desktop always had; Escape and
  keyboard focus are untouched.

  `/inbox` is built the same way and its scrim is buried the same way, but it
  has always shipped a reachable exit — the pinned, full-width, 44px-tall *Back
  to the question* button — so only its close glyph needed the touch size.

  The same pass raises the controls that were under 44px at phone width on both
  surfaces: the header's ghost and icon buttons, the language toggle, the nav
  pills, and `/inbox`'s *back to list* and ticket buttons, which are the phone's
  only navigation once the list pane is hidden. All of it is CSS inside the
  existing `@media (max-width: 760px)` blocks — every byte of both files outside
  those blocks is unchanged, and the desktop board and inbox render pixel-for-
  pixel identically at 1440 (0 of 1,296,000 pixels differ on either surface).

- **`/board` now shows when a question came back with a question.** A human can
  bounce a question back to the asking agent for more research instead of
  answering it; the question stays open and the ticket stays parked, but the
  ball is with the agent. The board reduced every open question on a ticket to
  `{count, blocking, advisory}` and threw `awaiting` away, so a bounced-back
  question was indistinguishable from one waiting on a decision: same
  `blocking` badge, same needs-a-human tint, and a drawer callout that promised
  "Answering resumes this ticket" when there was nothing to answer yet.

  A ticket whose open questions are *all* awaiting the agent now reads
  **in conversation** (DE *im Gespräch*) — a `⤺ in conversation` badge, a
  dashed card edge, and no attention tint — and its detail callout reads
  "In conversation — N question(s) with the agent / A human asked for more
  research. The agent owes the next reply." with a *Read the thread in the
  Inbox* link instead of *Answer in Inbox*. A pending decision still wins: a
  ticket carrying both a live thread and a question it is a human's turn on
  keeps reading `blocking`, because that is the reader's next action. In the
  board's question drawer the same question loses the needs-a-decision
  highlight and gains the same label.

  The term is deliberately the one the inbox's planned *In conversation* folder
  uses, and the partition is the same single field (`Question.awaiting`), so
  both surfaces can name and split the state identically. The "Ask a human"
  count is unchanged for now — it stays a plain open count until the inbox has
  a folder to move these into.

- **The `.handrail/` norm detectors can now see a committed branch.**
  `route-test-pairing` and `openapi-current` computed their changed set with
  `git diff --name-only HEAD`, which reports only *uncommitted* work — so on a
  finished branch, the moment you would most want a verdict, both exited 2 and
  reported `skipped`. That reads as "nothing to check" when it actually meant
  "I cannot see your change", and the workaround (re-apply the branch diff
  unstaged in a scratch worktree) was being reinvented per ticket. Both scripts
  now accept `HR_BASE`: unset it behaves exactly as before, while
  `HR_BASE=origin/main handrail run route-test-pairing openapi-current` compares
  `merge-base(origin/main, HEAD)` against the working tree — the branch's own
  changes plus anything uncommitted, and not whatever landed on `main`
  underneath it. Skips now say which kind they are: `SKIP[not-in-scope]` (your
  change did not touch that surface) versus `SKIP[no-changes-visible]` (the
  detector saw nothing and checked nothing — not a pass). A `HR_BASE` that does
  not resolve exits 3 (red) instead of silently degrading to "no changes". CI's
  shellcheck job now covers `.handrail/*.sh`, which it did not before.

- **`/board` declared `ticketFilter` twice in one object literal, and nothing in
  the repo could have noticed.** Both declarations had identical initialisers so
  JavaScript kept the last and the board behaved correctly — which is the
  problem: it survived several refactors because it is legal code that no
  runtime complains about. The duplicate is gone, and so is a dead
  `parseShareToken()` left behind when answer-link mode generalised it into
  `parseHashKey(key)`.

  The reason both survived is that `src/board.html` and `src/inbox.html` hold
  about 5500 lines of hand-written JavaScript that **no gate in this repo read a
  single line of**: clippy had the Rust, shellcheck the shell, Redocly the spec,
  and one test checked exactly one property of the two SPAs (DE/EN string-table
  parity). `scripts/lint-spa.sh` now extracts the single inline `<script>` from
  each page and runs a pinned eslint over it, in CI and as a `.handrail` gate.
  Findings are reported at the HTML file's own path and line number, so
  `src/board.html:874` is somewhere you can go.

  The ruleset is small and deliberately defect-only — duplicate keys, undefined
  names, dead bindings, unreachable code, parse errors — with no style rules at
  all, because a preset that also has opinions about this hand-written ES5 would
  bury the real findings until someone switched the job off. On the existing code
  it found exactly two things, both of them the ones fixed above, and nothing
  else; no suppressions were needed. This also retires the `new Function(...)`
  parse check agents had been hand-rolling per ticket: a parse error is an eslint
  error, so the lane subsumes it — and unlike a parse check it catches the
  duplicate key, which parses perfectly.

  Nothing is added to the pages themselves. The SPAs remain dependency-free
  single files; eslint is fetched per run by pinned `npx`, exactly as the spec
  job already fetches `@redocly/cli`, and no `node_modules` is committed. Run
  offline the script exits 2 — "cannot check", the posture `openapi-sane.sh`
  already takes on a missing PyYAML — rather than reporting a defect. CI's
  shellcheck job now globs `scripts/*.sh` instead of naming one file.

- **`↵` on a focused control in `/inbox` no longer answers a question you never
  read.** The global "answer the selected question" shortcut claimed Enter for
  the whole document and `preventDefault()`ed it. On a focused control that did
  not merely swallow the activation: it submitted an answer for whatever the
  *list* had selected, which after the post-answer auto-advance is the **next**
  open question. So pressing `↵` on the undo snackbar's focused Undo button —
  the most obvious recovery key, on the one control whose whole job is to take
  the last action back — recorded a second irreversible decision on an unrelated
  ticket, using the asking agent's `recommended` value (which arms the primary
  with no clicks at all), left the answer it was meant to cancel standing, and
  advanced the selection again so the next `↵` did it to the question after
  that. The same swallow reached every focusable control on the surface: the
  language toggles, and the question rows, where Tab+`↵` selected a row and
  answered it in one keystroke. Scoped by control kind rather than by
  special-casing Undo, so buttons added later are safe by construction; `j`/`k`
  and `↵` with focus on the document are unchanged.

- **`/inbox` says when an answered question left its ticket parked.** A blocking
  answer is meant to move the ticket back into the ready queue; when it cannot,
  the answer is still recorded and the ticket is left out of the queue, where no
  agent will pick it up. The inbox rendered nothing for that, so a stranded
  question looked exactly like a resumed one. The reading pane now carries a
  "ticket not resumed" block under the decision and the list row a `ticket
  stalled` chip, with the reason: the server's verdict on that answer (from the
  answer response, or from the `question_answered` event when the answer was
  given elsewhere), or — when another blocking question on the ticket is still
  open, so no resume was attempted — the barrier. Presentation only; the server
  has reported all of this since the resume fix.

- **The CLI no longer drops a write on a ten-second gateway blip.** `takomo`
  issued exactly one request per command, so a transient `502` from a proxy in
  front of the store — `/healthz` green throughout — failed the command
  outright. It failed *quietly*, too: the 502 body is a full HTML error page, so
  a command piped through `tail -1` showed a line of base64 font data and
  nothing that read as an error. A `comment` carrying a ticket's only proof of
  verification, or a `link --commit` closing the loop on a merged change, went
  missing with no sign.

  Requests are now retried with bounded exponential backoff and jitter
  (`TAKOMO_RETRIES`, default 3; `TAKOMO_RETRY_MAX_SECONDS`, default 20) — but
  only where a replay is provably harmless, decided per endpoint against what
  the store actually does: reads, `PUT`s, set-to-value `PATCH`es (`link`,
  `tag`), the Idempotency-Key'd ticket create, `claim` (an idempotent lease
  renewal), `archive`/`unarchive`, `dep`, and `ask` (the store dedupes an
  identical still-open question). A failure that provably never left the
  machine — DNS or connect refused — is retried for *every* verb, since nothing
  can have been applied.

  Writes that would double if replayed are never retried: `comment`, `next`,
  `promote`, `token create`, `share create`, and the question verbs. They now
  fail unmistakably instead — a banner, the request that failed, and, as the
  last line so it survives `2>&1 | tail -1`, either `THE WRITE DID NOT HAPPEN`
  with the exact command to re-run, or `THE WRITE WAS NOT CONFIRMED` telling you
  to check before you do. A non-JSON error body is now summarised
  (`502 Bad Gateway`, 1448 bytes of non-JSON) instead of dumped.

  `done` / `start` / `move` are rescued rather than refused: a transition cannot
  be replayed blind (the store has no `X -> X` edge, so a repeat 409s whether or
  not the first landed), so the CLI re-reads the ticket instead — already at the
  target means it landed and the command succeeds; still elsewhere means the
  replay is safe (takomo-6flc).

- **`/board` is usable on a phone.** The board had no responsive design at all:
  its only non-colour media query styled the Settings modal. On a 390px screen
  the header wrapped into five to seven rows — 200–260px of *pinned* chrome,
  about a third of the display — and behind it eight fixed 270px columns formed
  a ~2,270px strip you scrolled sideways with no snap and no idea where you
  were. Below 760px (the width `/inbox` already changes shape at) the columns
  now stack into **one vertical scroll**: the same states in the same workflow
  order, cards priority-sorted within each exactly as before, and an empty state
  collapsed to its heading line so all eight stay visible at a glance. Nothing
  scrolls sideways. The header collapses with it — `Refresh`, `Sign out`,
  `Settings` and `Ask a human` become glyph buttons that keep their names as
  `aria-label`s, and every filter moves behind one `Filters` control — taking
  the resting header from ~200–260px to **105px**, two deliberate rows. The
  ticket and tag-value typeaheads go full width inside that panel, so their
  popups track the control instead of hanging off it; a wide phone or a small
  tablet fits two cards abreast. The desktop board is pixel-for-pixel unchanged:
  the reveal is CSS-only, so no new code runs above 760px.

- **`/inbox` presentation: a stable undo button, a readable queue, one banner
  fewer.** Three faults reported from real use, all in the reading surface.
  The **undo snackbar** rebuilt its whole subtree once a second so that one
  digit of the countdown could change, which destroyed the Undo button under
  the pointer and under keyboard focus — a reader who tabbed to it lost focus
  within a second and could not activate a 30-second escape hatch at all — and
  replayed the toast's entry animation on every tick. Rows are now built when
  the set of pending answers changes and a tick only rewrites the label text,
  so the button survives; Undo stays bound to the question id, so overlapping
  windows still each take back their own answer (takomo-42o8). The **question
  list** ended each row with `agent:runner-2 · confirm · advisory`, three raw
  field values that scan as a debug dump: `kind` is gone (the reading pane
  already says it as the control you are handed), the asker is now framed —
  `asked by agent:runner-2` — because it is the only place the queue says who
  is blocked on you, and blocking-vs-advisory survives as a labelled chip
  because it changes what answering does (takomo-4s5z). And the **"This
  project expects answers in X" banner** above every answer area is gone; the
  project's `question_language` still reaches a human as the hint inside the
  answer placeholder and beside the project name in the picker, and still
  reaches agents as `language_hint` (takomo-9u54).
- **`transition.claim_required` no longer sends the caller into a dead end.** Its
  remedy was a flat `POST /v1/tickets/{id}/claim`, but a lease can only be taken
  in a state the workflow marks `claimable` — and the state you are stuck in when
  the lease expires mid-work (`in_progress`, `implementing`) is exactly one that
  is not. Following the remedy came back with `claim.state` — "state X is not
  claimable" — so the two errors pointed at each other and an agent trying to
  close a finished ticket had nothing left to try. The remedy now checks whether
  claiming works from here: in a claimable state it is still the plain claim,
  followed by the exact transition to retry with the new fence; in a
  non-claimable one it says outright that claiming would be refused, names the
  re-entry edges out of the current state that land somewhere a lease *can* be
  taken, and warns that going that way puts the ticket back in the ready queue
  where another worker could pick it up. `details` carries `claimable_states` and
  `reentry_states` so a machine reader need not parse the prose. The error code
  and status are unchanged, and whether an expired lease should block closing at
  all is a separate open question (takomo-jb5i).
- **Answering a blocking question resumes the ticket on the `simple` workflow —
  the half of ask-a-human that never worked on the default.** The resume looked
  only at `scope:human` transitions out of the parked state. `simple` — the
  workflow `takomo init` applies — has no `scope:human` edge anywhere, by
  design, so there was nothing to resume through: every answer recorded fine and
  left its ticket sitting in `blocked`, out of the ready queue, where no agent
  would ever pick it up again. Where the parked state has no human-gated exit at
  all, the resume now goes through any exit the answerer can take (scope
  satisfied, no claim, no guard) that leads somewhere non-terminal and
  unblocked, preferring a claimable `todo` — on `simple`, `blocked → todo`, back
  into the ready queue. A human gate that exists but is not takeable is still
  never routed around, so `factory-default`'s approval path is unchanged.
- **An answer that cannot resume its ticket now says so.** Previously it
  returned `200` with `resolved_to: null` and nothing else — the answerer was
  told the ticket would resume and had no way to learn it hadn't. The answer
  response now carries a `resume` block: `resumed`, the state it moved to, and,
  when it could not, a machine-readable `code`, a `message` listing every exit
  from the parked state with what each would take, and a `remedy` naming the
  next call. A comment lands on the ticket too, so it is visible to whoever
  reads the ticket rather than the response. The answer is still recorded either
  way. `takomo answer` used to print `ticket X resumed to 'blocked'` — the
  parked state read back as if it were the resume target; it now reports what
  actually happened, and prints the reason and remedy when the ticket did not
  move.
- **The `/inbox` answer button now says what it will do, and is live only when it
  can do it.** Three faults in the same control. It was armed before anything had
  been chosen, so it could be pressed in a state where it could only refuse; it
  is now inert until the question is actually answered — non-empty text for a
  `clarify`, at least one option for a `multi` choose, a preset or a non-empty
  "write your own" for a single choose, yes or no for `confirm`/`approve` — and
  it says why in a hint that reaches hover, keyboard and screen reader alike
  (`aria-disabled` plus an `aria-describedby` hint, rather than a native
  `disabled` that would drop it out of the tab order and explain nothing). With
  the **follow-up composer** open it kept submitting an answer, so a reader who
  had just typed a question back to the agent could resume the ticket by pressing
  the one big button under it; while the composer is open the primary now *is*
  the follow-up submit, by the `↵` shortcut as well as by click. And its label is
  simply **Submit** / **Absenden** instead of "Answer & resume" / "Answer &
  record" — advisory questions stay marked as advisory in the header, in the list
  and in the confirmation.
- **Reading the tracker over MCP no longer spends the write budget.** Every MCP
  frame is a `POST /mcp`, and the rate limiter classified writes by HTTP method,
  so `takomo_show`, `takomo_list`, `takomo_ready` — and even `tools/list` — each
  debited the token's 120 writes/minute and, on exhaustion, came back with a 429
  saying the token had "exceeded its write budget", sending an agent hunting for
  writes it never made. An agent could rate-limit itself out of the tracker with
  zero mutations. The budget is now charged per tool call: the read-only tools
  are free (as `GET /v1/...` already was), the handshake and `tools/list` are
  free, and every mutating tool debits exactly one write. The 429 also says what
  it means — it names the budget, states that reads are free and still work, and
  carries a `remedy`.
- **One `/v1/export` no longer stalls every claim and heartbeat in the process.**
  Reads and writes shared a single SQLite connection behind one mutex, so any
  long read — an unfiltered export, `/v1/metrics`, a project roadmap, a
  transitive dep graph — froze every claim, transition and heartbeat for its
  whole duration. Measured on 8k tickets: a claim that normally takes 0.2ms took
  **104ms**, about 80% of the export. Reads now run on read-only companion
  connections (WAL makes them concurrent with the writer, and each read still
  gets one consistent snapshot), and the scan-shaped endpoints run off the async
  runtime. Same claim during the same export: **under 10ms**, with roughly twice
  as many claims completing while it runs. There is still exactly one writer —
  the guarantee that a ready ticket goes to exactly one claimant is untouched.
- **An answer link is now spent in the same transaction as the answer it
  carries.** The `tka_` token you hand an outside expert is single-use, but the
  write that marked it used committed *after* the answer, in a transaction of its
  own. Single-use still held — a second attempt found the question no longer open
  — yet it held by accident of unrelated bookkeeping rather than by the
  transaction that claimed it, and the observable behaviour was wrong in a way
  the expert saw: because the question's resolution sweep revoked the link before
  the follow-up write could mark it used, someone who reloaded a link they had
  just used was told it *"has been revoked"*. The spend is now the first thing the
  answering transaction does, so it is what orders simultaneous submissions on
  one link (two tabs, a double-click, a forwarded message): exactly one is
  applied, and the rest get `410` with the new `answer_link.spent`, telling the
  reader another answer landed first and nothing of theirs was recorded. A link
  spent by its own answer now correctly reports itself used, a revoke arriving
  while an answer is in flight wins, and a rejected answer — a bad option, a
  missing expert scope — rolls the spend back with it, so a link is never burned
  by an attempt that did not land.
- **The inbox no longer goes silent while you are answering.** `/inbox` defers a
  batch of events while a human is mid-answer — but it advanced the event cursor
  *before* deciding to defer, so the skipped batch was consumed and never fetched
  again. Since focus inside the reading pane counts as busy, and every answer now
  holds a 30-second undo window, a normal pass through the queue kept it busy
  continuously: new questions never arrived, answered ones never left, and only a
  reload recovered. The cursor now advances only over events that were actually
  applied, so a deferred batch is re-delivered on the next idle poll.
- **Committing one pending answer no longer resurrects the others.** When an undo
  window closed, `/inbox` wrote that answer and reloaded the list — and the reload
  replaced every other still-pending answer with the server's view, where those
  questions are of course still open. They popped back into the Open folder and its
  count while their own snackbars were still counting down, one of them was
  auto-selected with its full answer UI, and pressing "Answer & resume" on it did
  nothing at all, because that answer was already queued. Answering two questions
  within 30 seconds of each other was enough. Every reload of the question list —
  a closing window, a project switch, a withdraw, a reopen, a follow-up, a poll —
  now re-applies the answers still inside their undo window over the fresh list, so
  the queue never asks again for work you have already done. Undo keeps working
  across such a reload, and puts back what the server currently says.
- **`DELETE /v1/projects/{project}` 500'd on any project that had ever carried a
  question, a tag, an answer link or a promotion.** The cascade cleared tickets,
  comments, deps, events and idempotency records but not `questions`,
  `question_messages`, `answer_grants`, `tags` or `promotions` — each of which
  holds a real foreign key into `questions`, `tickets` or `projects`, so SQLite
  aborted the transaction and the caller got an opaque internal error. All five
  are now cleared in foreign-key-safe order inside the same transaction, and the
  `project_deleted` audit event reports the rows removed per table.
- **Every answer gets its own undo window**, and an answer is confirmed in the
  data as it is given rather than 30 seconds later or in a floating toast.
- The reading pane keeps its scroll position when a choice is selected.
- A clear answer→next transition, with the custom answer always available.
- Security, correctness, a11y and i18n findings from two review rounds:
  advisory resume, dependency scoping, and answer-grant revocation.
- `spec/openapi.yaml` was not a valid 3.1 document in two ways invisible to a
  human reader; CI now validates it against the schema and loads it from Python
  and Ruby.
- `.mcp.json` is now gitignored. `claude mcp add --scope project` writes a
  bearer token into that file in plaintext, and nothing was stopping a `git add .`
  from committing it. The README now says which scope does this next to the
  command it documents.

### Changed

- **`/board`'s ticket filter is a search box, not a 130-option dropdown.**
  The "All tickets" `<select>` listed every ticket in the project; past a
  hundred, the options are long, truncated and near-identical, so picking one
  was guesswork. It is now a typeahead: type to filter on **id or title** (any
  order — `sweeper fence` and `fence sweeper` find the same ticket), with
  id-prefix matches ranked first so typing an id lands on that ticket. Built by
  DOM construction with no library, following the ARIA combobox pattern, so it
  stays fully keyboard-operable the way the `<select>` was for free: Tab to
  reach it, arrows to move, Enter to pick, Escape to dismiss without changing
  the filter, and a `×` button in the tab order (or Backspace on an empty box)
  to clear. The popup always says where it stands — the match count, `no match
  for "…"` when nothing hits, and a `keep typing` hint past 60 rendered rows so
  the DOM cost stays flat as a project grows. Every visible string is re-read on
  render, so the DE/EN toggle repaints the control immediately instead of
  leaving a stale label behind.
- **`/board`'s tag-value filter is the same typeahead.** The second half of the
  kind-then-value tag picker has the ticket filter's problem over a real
  registry, so it reuses the ticket filter's control rather than growing a
  second one — the two lists differ only in what a row is made of. Search hits
  either half: `ada` finds the handle, `Lovelace` the registry's display label.
  The *kind* picker stays a `<select>`: a project has a handful of kinds, they
  fit without truncating, and "any tag of this kind" is a useful filter in its
  own right, so there is nothing there to search.
- **Question comments name the question by id instead of restating its title.**
  Every comment a question mirrors onto its ticket used to open by quoting the
  question title — `Human answered "OK to drop table billing_v1?": yes /
  approved` — which is pure repetition in the one view where the title is
  already on screen (the board drawer, the inbox), and it pushed the part
  carrying new information to the right. The title was never a real archive
  either: a ticket view lists only *open* questions, and a title alone never
  carried the body, the options or the thread. So all five bodies now read
  `… q-9f3ka2xz: …` — answer, reopen, human follow-up, agent reply and options
  revision — and the id resolves to the whole question (`GET
  /v1/questions/{id}`) for a reader coming back to the ticket months later. Not
  a contract change: comment text is prose, no `/v1` field moved.
- **`workflows/` is the one place a shipped workflow is defined.**
  `factory-default` moved out of a `serde_json::json!` literal in
  `src/workflow.rs` into `workflows/factory-default.yaml`, embedded with
  `include_str!`. The copies that remain for other reasons — the block in
  `spec/workflow-format.md`, and the CLI's offline fallback for `simple` — are
  pinned to the files by unit tests. `backlot.yml` and the `Dockerfile` learned
  that `workflows/` is a build input.
- Removed `prompts/spec-agent.md`; nothing referenced it.
- **`setLive` is gone from both web surfaces.** 0.3.0 dropped the navbar live
  indicator from `/board` and `/inbox` but kept `setLive` as a null-tolerant
  no-op so its call sites did not have to change — leaving a function named like
  a status indicator that wrote to `#live-dot` and `#live-text`, neither of which
  exists. Every call was silently doing nothing, which is exactly the trap the
  removal was meant to avoid elsewhere. The function, its 20 call sites (8 in
  `/inbox`, 12 in `/board`) and the orphaned `.dot` CSS are deleted from both
  files. No user-visible change — the calls already did nothing, and the strings
  they passed were hardcoded English that never reached a `STR` table — but the
  next reader of the poll loop no longer has to discover that for themselves.
- **Dependency hygiene.** Workflow YAML is now parsed by `serde_norway` instead
  of `serde_yaml`, which upstream archived in March 2024 and publishes as
  `0.9.34+deprecated`; it is the same 0.9 API, keeps the MIT OR Apache-2.0
  licensing, and drops the archived `unsafe-libyaml` backend along with it.
  Strict workflow parsing is unchanged — a typo like `require:` is still a hard
  error naming the field and the line, not a silently dropped approval gate.
  `tower` and `tokio-stream` were declared with zero call sites and are gone
  (they remain transitively via axum/rmcp, so the build is unaffected), and
  `tokio` now names the five features this binary actually uses instead of
  `full`, dropping six crates from the lockfile. A new CI job runs
  `cargo-machete` so an unused dependency cannot creep back in.

## [0.3.0] — 2026-07-25

Human-in-the-loop, refined. The ask-a-human inbox is rebuilt to the "Aquarelle"
design with a **DE/EN** language toggle, a **trailing-undo** answer flow
(auto-advance + a 30s background undo), an **ⓘ ticket-context** popover/drawer,
and a **follow-up thread** (bounce a question back to the agent for more
research). Questions carry richer, decision-ready fields — **per-option
descriptions, `confidence`, a recommendation rationale, a `summary`** — and can
be **multi-select** or **reopened** (a conditional undo once the 30s window has
passed). Work can be **promoted** to a named stage (prod/staging/published/…),
and the takomo **octopus** is the site favicon.

### Added

- **Multi-select `choose` questions.** A `choose` question can set `multi: true`
  (with an optional `recommended_multi` set) so a human can pick several options;
  the answer is the chosen array. Exposed on `POST /v1/questions`, `takomo_ask`,
  and `takomo ask --multi --rec-multi …`. The `/inbox` renders checkbox-style
  options for it.

- **Reopen an answered question** — a conditional undo beyond the inbox's 30s
  window. `POST /v1/questions/{id}/reopen` / `takomo reopen` / `takomo_reopen`
  (human scope; matching expert for `approve`) returns the question to `open` and
  re-parks the ticket, but only while the answer isn't yet in use — refused with a
  teaching `409` once the ticket is claimed, has moved past the state it resumed
  into, or is archived (re-ask instead). The `/inbox` shows a **Reopen** button on
  answered questions. Emits `question_reopened`.

### Changed

- **Inbox answering is now trailing.** Answering completes the item optimistically
  and jumps straight to the next open question (with a small micro-animation); the
  30-second undo runs in the background. Only **Undo** brings the item back to its
  former status and re-selects it. Committing a new answer flushes the previous
  one. The navbar drops the `live` indicator and (on the inbox) the unused `mine`
  toggle; the project selector is restyled and now **remembers the last-selected
  project**. The inbox navbar wraps instead of overflowing on narrow screens.

### Added

- **Richer question fields (make the inbox fully data-driven).** Questions gain
  optional, additive fields the redesigned inbox renders: per-option
  descriptions (send `options` as `[{value, desc}]`, or a parallel
  `option_notes`), `recommended_note` (the rationale), `confidence` (1–4, drives
  the recommendation gauge), and `summary` (list preview). Exposed on
  `POST /v1/questions`, `takomo_ask`, and `takomo ask`
  (`--option-desc`/`--rec-note`/`--confidence`/`--summary`). The ask response now
  returns a non-blocking **`hints`** array telling the agent what optional field
  would improve the card — it never fails the ask. Agent guidance (MCP tool
  description, plugin skill) updated to write decision-ready questions.

- **Inbox & board redesign (Aquarelle prototype) + DE/EN localization.** `/inbox`
  is rebuilt to the latest design: a **DE/EN language toggle** (whole UI
  localizable, remembered per device, defaults to the browser language), an
  urgency-**grouped** question list with rank-glyph headers and preview lines, a
  reading pane with an **ⓘ ticket-context popover** and a slide-over ticket
  drawer, a **"Ticket update" timeline** for the follow-up thread, kind-adaptive
  answer cards (choose/clarify/confirm/approve) with the recommendation
  highlighted, a follow-up **compose** control with a thread-count badge, a
  **withdraw confirmation** dialog, and a single-click **Answer** guarded by a
  centered 30-second **undo snackbar** (replacing the press-twice arming). The
  board gains the same DE/EN toggle, a 4-bar priority gauge on cards, and
  localized chrome. Behavior (token gate, live polling, share/answer-link modes,
  the ask-a-human drawer, the follow-up loop) is preserved.

- **Ticket promotions.** Record that a ticket's work reached a named
  target/stage — `POST /v1/tickets/{id}/promote {target, url?, ref?, note?}` /
  `takomo promote` / `takomo_promote`. `target` is free-form ("staging",
  "production", "published", "delivered", …) so takomo isn't tied to software
  deployment. Append-only history (`GET /v1/tickets/{id}/promotions`,
  `?include=promotions`); the latest per ticket (`GET /v1/promotions?project=`)
  badges the board card, and the detail drawer shows the full history. Emits a
  `ticket_promoted` event; `takomo_show` surfaces promotions to agents.

- **Ask-a-human follow-up thread.** A human can now bounce a question back to the
  asking agent for more research *before* answering, tracked as a clear
  thread. `POST /v1/questions/{id}/followup` (human) records the request and sets
  `awaiting: agent`; the agent replies with `POST /v1/questions/{id}/reply` /
  `takomo_reply` (write), flipping `awaiting` back to `human`. The question stays
  open and a blocking ticket stays parked throughout. `GET /v1/questions/{id}`
  now returns the `thread` and `awaiting`; `takomo_show` surfaces the thread on
  open questions so an agent's work-loop picks up the request. Surfaced in the
  `/inbox` reading pane (Ask-for-more action + thread) and the board drawer.
- `/board` and `/inbox` re-skinned to the "Aquarelle" design (structure +
  palette), and the takomo octopus mark now ships as the site favicon.

## [0.2.0] — 2026-07-24

First tagged release: a single-binary, self-hostable, hosted task tracker that
every AI agent, orchestrator, and human on a project talks to over HTTP. The
headline addition since the initial public release is the **ask-a-human board**
(questions, expertise routing, notifications, per-question answer links, and a
dedicated `/inbox` triage page); the rest below is the baseline it builds on.

### Server

- Single Rust/axum binary over SQLite (WAL) — HTTP server plus `token` and
  `project` admin subcommands.
- Hierarchical tickets (`epic` → `task`/`bug`/`subtask`) with single-parent
  trees, `blocked_by` dependency edges, labels, and free-form namespaced JSON
  metadata.
- Per-project, server-enforced state machine with a configurable workflow
  format; illegal transitions return a teaching `409` (current state, allowed
  transitions, and a remedy) written to be read by an LLM.
- Atomic claim/lease with a monotonic fencing token so exactly one worker owns a
  ticket; expired leases return the ticket to the ready queue.
- Append-only event log with a durable `?since=<seq>` cursor and an SSE stream.
- Ready queue (`GET /ready` peek, `POST /ready/claim` atomic take) driven by
  dependency readiness.
- Bearer-token auth, scoped (`read`, `write`, `human`, `autoland`, `admin`) and
  SHA-256 hashed at rest; token minting over both the server CLI and an
  admin-scoped HTTP surface.
- Read-only web board at `/board`, plus scoped, expiring share links.
- Ask-a-human board: agents raise a typed question (`confirm`/`choose`/
  `clarify`/`approve`) with `POST /v1/questions` / `takomo ask` / `takomo_ask`.
  A **blocking** question parks the ticket and releases the lease
  (block-and-resume); the ticket resumes only when all its open blocking
  questions are answered (a barrier). An **advisory** question records a routed
  decision with no state change — for epic-level or strategic calls. A
  `human`-scoped answer records the reply and, for a blocking question, performs
  the ticket's human-gated resume transition; `approve` questions additionally
  require the answerer to hold the matching `expert:<tag>` scope. Questions route
  by expertise tag (free-form `expert:<tag>` scopes), surface on a `/board`
  inbox with an unread badge, and support deadlines with an `on_timeout`
  fallback swept alongside leases. Optional outbound notifications (Slack /
  generic webhook / SMTP email) via `TAKOMO_NOTIFY`, off unless configured.
  A per-project **question language** (`takomo project language` /
  `PUT /v1/projects/{id}/language`) nudges agents to phrase ask-a-human
  questions in a set language (e.g. German for a revamp project) — surfaced as a
  `language_hint` on the MCP work-loop tools, `question_language` on
  `takomo_workflow`, in the `takomo_ask` result, and as an inbox reminder. Soft,
  never enforced.
  Per-question **answer links** (`POST /v1/questions/{id}/answer-link` /
  `takomo answer-link` / `takomo_answer_link`) mint a scoped, expiring,
  single-use `tka_` token so an outside expert can answer one question via
  `/board#a=<token>` (a distinct `/v1/answer/self` auth path) without holding a
  standing token. See docs/ask-a-human.md.
- Archive support (additive, non-destructive startup migration).
- JSONL export/import with idempotent re-import; importers for takomo, beads,
  and beans.
- `/healthz` as the only unauthenticated endpoint; refuses non-loopback binds
  unless `TAKOMO_ALLOW_PUBLIC_BIND=1`.

### Clients

- `takomo` — a self-contained `bash` + `curl` + `python3` CLI over the REST API,
  with `takomo init` one-command repo onboarding and local fence tracking.
- Claude Code plugin (this repo doubles as the plugin marketplace): bundles the
  takomo skill and a remote MCP server declaration for the hosted endpoint.
- Model Context Protocol (MCP) server for agent harnesses.
- Agent skills for using the store as a source of truth and for onboarding a
  repo.

### Deployment

- Render Blueprint (`render.yaml`) with a persistent disk and health check.
- Portable `Dockerfile` for VM / self-host deployment.
- Prepared (opt-in) Litestream continuous backup to S3-compatible storage.
