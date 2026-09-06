# web — Takomo's browser surfaces

One source, **two builds**, because two different consumers want different things:

| build | command | output | consumed by |
|---|---|---|---|
| **app** | `npm run build` | `dist/index.html` + `dist/assets/{app,vendor,runtime}.js` + `dist/assets/app.css` | the Rust binary, via `include_str!` |
| **library** | `npm run build:lib` | `dist-lib/index.js` + `index.css` + `index.d.ts` | `claude.ai/design` (the design-sync skill) |

The app build is what ships. The library build exists because a design system is
consumed as components with a props contract, not as a bundled application —
and keeping it green is a constraint with teeth: a component that cannot be
built there is one that reached into page state instead of taking props.

## Status: complete — every surface is built from here

Nine routes now: `/board`, `/inbox`, `/documents`, `/initiatives`, `/mindmaps`, `/schedules`, `/verification`
and `/environments`. Most are one bundle behind a router, so moving between them
costs nothing. `/documents` and `/mindmaps` are code-split, because both pull the
CRDT runtime and the editor pulls Tiptap on top — the size budget measures FIRST
LOAD for that reason, and separately caps the heaviest lazy chunk.

`src/api/mod.rs` serves every page from `web/dist/`. Every hand-written page is
gone, and with them `src/spa-common.js`, `scripts/lint-spa.sh`,
`scripts/spa-eslint.config.mjs` and the `spa-lint` gate — they existed only to
read and guard the JavaScript inside those files.

| # | page | state |
|---|---|---|
| 1 | **initiatives** | **ported** — verified against a running server: token gate, list, detail, markdown, DE/EN, and an entry appended through the UI |
| 2 | **schedules** | **ported** — verified live: proposal banner + nav badge, cadence lines, the 8-cell occurrence strip, an idempotent `run`, and the create dialog's conditional cadence fields |
| 3 | **inbox** | **ported** — verified live: folder rail + counts, four question kinds, recommendation pre-arming, follow-up thread, answer→undo→commit, withdraw/reopen, answer-link minting, and the ticket typeahead matching on titles |
| 4 | **board** | **ported** — three modes in one route: the board, `#s=` read-only share, `#a=` single-use answer link. Verified live, including the grant modes |

Each port flips exactly one `include_str!` path, so the surfaces can be replaced
one at a time with no big-bang cutover.

## Two altitudes on /board

`/board`'s view toggle is a different axis from the three modes above: those are
about **who you are** (a board token, a share, an answer grant), this is about
**what you are looking at**.

- **Board** — the kanban. Where is each ticket. `by epic` groups the same columns
  under epic headings; it is still a ticket board.
- **Epics** — one row per epic, no columns. Where is each epic, which initiatives
  it belongs to, who holds it, and whether it is moving. It reads
  `GET /v1/projects/{p}/roadmap` and is fetched only while open, because that
  endpoint runs a query per epic; it refreshes when the event poll finds
  something rather than on the poll's own four-second tick.

That poll is also what refreshes the tickets themselves and the rail's badges, so
one mistake in reading its response stops all three at once — and stops them
*silently*, because a page that never hears about a change looks exactly like a
project where nothing happened. `api<T>` is an unchecked cast of a JSON body, so
`EventPage` naming a field the server does not send compiled and typechecked
happily; the array is **`events`**, per `/events` in `spec/openapi.yaml`, and
`src/lib/board.test.ts` pins that against a literal copy of the wire payload
rather than a fixture built from the type it is testing.

Rows are in the server's order — creation order — deliberately. Ranking them here
would be the page inventing a priority the API does not have; the counted
attention strip at the top is the fast read instead. `docs/epic-claims.md` owns
why a held-but-idle epic is worth a surface at all.

## Commands

```sh
npm install
npm run dev          # vite on :5173, /v1 proxied — NO Rust rebuild in the loop
npm test             # vitest
npm run check        # tsc --noEmit
npm run lint         # eslint, defect rules only
npm run build        # every page
npm run size         # gzip budget: first load, and the vendor chunk
npm run build:lib    # the component library + .d.ts
```

Point the dev proxy at a running instance: `TAKOMO_DEV_API=http://127.0.0.1:<port> npm run dev`
(get the port from `backlot up`).

## shadcn

Components are **copy-in**, in `src/components/ui/`. Add more with:

```sh
npx shadcn@latest add <component> -y
```

Two things to know before re-running `init`:

**It will re-append its default palette and font.** `shadcn init` writes a
neutral-grey oklch palette into `src/styles/globals.css` plus
`@import "@fontsource-variable/geist"`. Both are removed here on purpose — the
font is 228 kB inlined into all four documents, and the palette **collides with
Aquarelle on three names**: `--accent` (deep blue here, hover wash there),
`--muted` (text here, background there) and `--border`. Left as generated,
`var(--accent)` silently turns grey everywhere. The file documents the
reconciliation; re-apply it if you re-init.

**Theming maps in `@theme inline` only.** shadcn tokens are aliases of the
Aquarelle variables (`--color-primary: var(--accent)`), never a second palette,
so `tokens.css` stays the one place a color is defined and dark mode needs no
duplication. The dark variant is bound to `prefers-color-scheme`, not shadcn's
`.dark` class, because these pages follow the OS and have no toggle.

## The responsive contract

**One breakpoint: `md` (768px). It means "phone or not".**

Takomo has no tablet-specific design, so `sm:` and `lg:` exist in Tailwind but
should not appear in new code — a third layout nobody looks at is worse than two
that are checked. `web/src/hooks/useIsPhone.ts` mirrors the same line in JS, for
the handful of cases where the difference is structural rather than visual.

Design mobile-first where it is free: an unprefixed class is the PHONE style, and
`md:` adds the desktop one. That ordering matters, because the failure mode is
always the same — a desktop-shaped value with no mobile fallback:

| write | not |
|---|---|
| `w-full md:w-72` | `w-72` |
| `grid-cols-1 md:grid-cols-[180px_320px_1fr]` | `grid-cols-[180px_320px_1fr]` |
| `h-dvh` | `h-screen` |
| `max-w-[calc(100%-2rem)] sm:max-w-140` | `max-w-140` |

Four eslint rules enforce the left column (`eslint.config.js`). They exist
because every one of those right-hand values shipped and broke a phone: the grid
resolved its third column to **0px** at every phone width, `w-72` columns made a
2400px strip in a 375px window, `h-screen` is the *large* viewport on mobile so
the page bottom sat under browser chrome, and `max-w-140` silently deleted
shadcn's mobile inset via tailwind-merge. All four look correct in source, which
is why they are lint rules rather than review notes.

`max-w-*` is deliberately NOT restricted: it is a cap, so on a narrow screen it
does not bind and cannot overflow.

**What lint cannot catch.** jsdom has no layout engine — `getBoundingClientRect`
returns zeros — so no vitest test can see a collapsed column or a dropdown
rendered off-screen. Those need a real browser. Until there is one in CI, a
change that alters layout should be checked by hand at 375px; note that Chrome's
macOS window will not go below 500px, so drive the app inside a 375px iframe.

## Decisions worth knowing before editing

**Navigation is a left rail, not a header strip.** `AppShell` wraps every
surface: `NavRail` on the left, the page's own `AppHeader` and body to the right
of it. The header used to carry the brand, five surface names, the project
picker and up to four action buttons in one row — the nav was already scrolling
sideways to hide what did not fit, behind an edge that signals nothing. The rail
gives the surfaces their own axis, so a sixth costs vertical space nobody is
short of. Two states, toggled by the icon at its top: expanded (icon + label,
`w-56`) and collapsed (icons only, `w-14`). The choice is a viewer preference,
so `useNavCollapsed` persists it per origin and every surface reads the same key.

**The project picker is in the rail too**, because it is not about the current
surface — it SCOPES all of them, and a control every page obeys belongs with the
navigation rather than in each page's own toolbar. It stopped being a native
`<select>` in the move: a `<select>` cannot be searched, and an install with
fifty projects turns it into a scroll hunt. `ProjectPicker` is a trigger plus a
popover with a search field, and it collapses to the project's initial.

It is deliberately NOT `Typeahead`. That one is a filter — always showing its
input, sized for a toolbar; this is a navigation control that must render as a
40px square. They share the part worth sharing, `lib/typeahead`: ranking,
truncation, and counting the total BEFORE the slice so a footer cannot claim a
truncated list is complete. Fork that logic and the ranking rots in whichever
copy nobody maintains.

`/board` passes no `all` label, so it offers no "All projects" entry — a kanban's
columns come from ONE project's workflow and two projects need not agree on their
states. The other three surfaces offer it.

Sign-out lives at the bottom of the rail, with the actor and its role. It used
to be one more icon button beside "refresh" on every page — two adjacent glyphs,
one harmless and one that ends the session.

On a phone the expanded rail would take 224 of 375 px, so there it **overlays**
the content with a backdrop instead of pushing it, a spacer holds the collapsed
strip's place in the flow, and following a link closes it. That is a structural
difference rather than a visual one, which is why it reads `useIsPhone` instead
of taking a `md:` prefix.

**/documents is the PLAN, not a filing cabinet.** A project has one plan, and
the map and this page are two renderings of it: a node is a section, its title
is the heading, its depth is the heading level, tree order is reading order
(`spec/one-model-two-views.md`). The page opens the MAP's sync session — one
socket for the whole view — and every section is an editor bound to that node's
own `prose` fragment, which is available rather than hoped for:
`@tiptap/extension-collaboration` resolves
`this.options.fragment ? this.options.fragment : document.getXmlFragment(field)`.

Three consequences worth knowing before editing `pages/documents/`:

- **Structure comes from a light read.** With prose inside the nodes, "the
  document changed" now fires on every character anybody types, and the outline
  changes for none of them. `readPlanTree` reads only id/parent/order/title, and
  `sameTree` keeps the projection when the shape did not move. Without that pair
  a remote keystroke re-renders every section — and a section is a mounted
  editor.
- **Editors are mounted only near the viewport.** The cap is 500 sections and
  500 ProseMirror instances is not a thing to do to a browser. An offscreen
  section renders its prose as plain text, which is also what holds its height,
  so nothing jumps as editors mount behind you. jsdom has no
  `IntersectionObserver`, so there everything mounts — which is what makes the
  binding testable at all.
- **A heading is read-only here.** The title caret lives on the canvas; two
  carets on one `Y.Text` in two layouts is a fight, not a feature. The section
  offers "show it on the map" instead, which hands over by link (`#m=…&n=…`) and
  the canvas selects and centres what arrives. The other direction is the map's
  ⌘K "read it as the plan" and the `≣ Plan` button, which land on `#n=…`; both
  links are built and read in `lib/plan-url.ts`, and both are honoured once and
  then cleared, because a cursor kept in step between two views drags a reader
  back every time the other one moves.

**An agent proposes to a SECTION; a person accepts it there.** Proposals live in
the map's own document, in the top-level `proposals` map, each record carrying
the `node` it is about — so one an agent writes appears in an open browser at
once and survives a disconnect. `lib/plan-proposals.ts` groups them by section
and counts what is waiting; the outline and the section header both say so
before anything is opened, and a folded branch reports what is waiting inside
it. Accepting applies the ops to that section's fragment THROUGH ITS EDITOR,
because markdown becomes nodes in the editor's exact schema and only the editor
has it — which is the same reason the server refuses to construct them
(`src/api/docprops.rs`). Three rules the panel keeps and a change here must not
break: the blocks a pending proposal touches are marked with a ProseMirror
**decoration** and never a mark (a mark would be content, which is the very rule
the highlight illustrates); an op the browser cannot apply after all is
**reported**, not dropped quietly; and a decision is **recorded** — a rejected
proposal stays visible as rejected, because it is a signal about the plan
somebody was wrong about. A reader gets the proposals and no buttons.

What the page shows beside the prose is where each section STANDS — agreed,
changed since somebody agreed, or never read — plus its history. Both come from
SQL rather than from the CRDT (`src/store/trace.rs`): the update log is the
mechanism that rebuilds text and is rewritten by compaction, so asking it who
changed §2.1 is asking a storage format a question about people. The trace is
sparse by contract, which is why an edit is filed only once it SETTLES — on blur
or after a pause — and never per keystroke.

The conversion this replaced — `POST /v1/mindmaps/{id}/documents`, the ⌘K "write
this map up", and `lib/document-outline.ts`'s folder model — is gone. It copied
the map into `documents` rows, so a node's notes and its document's prose were
two places one paragraph lived and disagreed after the first edit.

**On /mindmaps the canvas IS the page, and ⌘K is the chrome.** A project holds
exactly one brainstorm, so the rail that listed maps was a list of one and the
rail that listed projects was navigation competing with the thing being
navigated; both are gone, and so is the project picker in `NavRail` on that route
alone. The header keeps what a shared document needs — its title, whether this
browser is connected, who else is in it — and everything else is a command.

**SELECTING A NODE IS NOT OPENING IT.** Selection used to expand the card into a
300×320 reading panel drawn over its neighbours, so every click on the map threw a
panel across it whether or not anybody had asked to open anything. Selection now
highlights the node, brings up the pill and the `+`, and does nothing else. Every
card is drawn at `NODE_WIDTH`×`NODE_HEIGHT`, so `lib/mindmap-layout.ts` is the only
thing that decides where anything is and there is no size that varies by state.

What a card carries is a title and the always-on marks that say WHERE the
substance is — `≋` notes, `¶ n` relations, `→` what it became, `⌁` an agent wrote
it, the trust mark, `⊞ n` for a folded branch — plus one quiet line of the notes'
first sentence, or the titles a fold is standing in for. That is what turns a map
of thirty labels into a map of thirty thoughts while keeping 500 of them readable.

**Reading a thought properly is `NodeDialog`**, which already existed, is already
the editing surface and already has a read-only state; one surface rather than a
canvas panel and a dialog that overlap. It holds who wrote it, what it became, the
notes, kind, shape, colour, edge label, the reviewed flag, this node's relations,
a question's answer, and a read-only attachment list with a button through to
`AttachmentsDialog`. It opens from the pill's `node.open`, ⌘K, the right-click
menu, and the `✎` on an `Outline` row — deliberately NOT from selection and not
from double-click. It commits a field when you leave it and when it closes — no
save button, for the reason `/documents` has none — and closing hands the keyboard
back to the canvas.

**THE ONE TEXT CARET ON THE CANVAS IS A TITLE.** A modal per new thought is too
heavy for the ten minutes a brainstorm is for, so creating a node — Enter, Tab, the
`+`, `+ Branch`, ⌘K's add verbs, a double-click into empty space — makes it appear
where it will live showing only its title, with the caret in it. Enter commits and
KEEPS the node selected, so the next Enter makes the next sibling and the fast loop
is a loop; Tab commits and goes a level deeper. Escape abandons: a thought that was
never named is removed, because the gesture made a box rather than a thought, and a
node that had a title keeps it. Emptying an existing title and committing is still
a deletion, behind `PruneDialog`'s two questions. Renaming uses the same caret from
F2, double-click, the pill, the menu and ⌘K, so there is exactly ONE way to change
a title — which is why `NodeDialog` shows it as a heading and has no title field.
The four endings are `lib/mindmap-naming.ts`, pure and tested, because jsdom can
prove nothing about a canvas. `NodeNameInput` stops every event that would
otherwise pan, zoom, fold or prune while somebody is typing, and a read-only token
never gets a caret from any entrance.

**The affordances live in the margin and appear on selection.** Unselected, a
node is a title, its marks, and a count badge if anything is attached — nothing
else. Hovered, a `+` appears at its right edge and adds a child. Selected, a
small pill floats above it with THREE OR FOUR verbs and no more: open, rename,
fold/unfold, and relate. Which four is a pure function (`pillVerbsFor`), tested
rather than reviewed, and the choice is by what the other affordances already
cover — `+` adds, the badge attaches, right-click removes, and ⌘K carries the
long tail. A fifth verb wanting onto the pill is the signal it belongs in ⌘K,
because a pill that lists everything is the side panel coming back in a rounder
shape. Opening survives a read-only token because it is a read; fold survives
because it is per-viewer and never touches the document.

**Right-click opens `NodeMenu` and NOTHING else.** It does not select the node,
because selection is what brings the pill up and doing two things when one was
asked for is the bug — so `onPointerDown` ignores every non-primary button, and
the menu carries the id of the node under the pointer rather than reading
`selected`. Its verbs are therefore `menuItemsFor(id)`, and running one passes
that id back as an explicit target. It is not right-click only: Shift+F10 and the
ContextMenu key open the same menu anchored on the selected node, because a menu
reachable by one gesture no keyboard has is a set of commands a keyboard user does
not have. Deleting a branch still goes through `PruneDialog`'s two questions from
every entrance.

**Attachments are a badge and a dialog, not chips on the card.** The badge shows
the current count on EVERY node that has one, which is how you see there is
something there; `AttachmentsDialog` behind it lists, adds, corrects and removes.
Correcting is an in-place `updateAttachment` rather than remove-then-add, because
the id is what every other peer is holding.

**Dropping something on a node attaches it, and an attachment is a POINTER.**
A dropped file contributes its NAME and a kind inferred from its extension and
nothing else — no bytes, because bytes in a CRDT update log are bytes every peer
replays on join — so its `ref` is empty and its gist says the file still lives
where it was dragged from. Dropped text or a URL becomes a `link` carrying the
text in `ref`. `lib/mindmap-attach.ts` is the whole inference, pure and tested;
`Canvas` only reduces the browser's `DataTransfer` to names and text. The default
is prevented on the WHOLE canvas, not just on a node: a file dropped in empty
space would otherwise navigate the browser to it and throw away the map, the
connection and whatever anyone was typing.

The phone gets all of that as plain buttons on each `Outline` row — attachments
(with the count), open, rename, add after, add underneath, detach, remove —
because every canvas affordance here is pointer-driven and a phone has neither
hover nor a right button. It carries the same one caret the canvas does and no
more: a row being named shows a title caret in place, `Aa` opens it on a row that
already has a name, and `✎` opens the same `NodeDialog` for everything else.

**Every card carries a line of substance, and folding SUMMARISES.** A card used to
be a title and its marks; it now also carries one quiet line saying
what the node actually says — the first sentence of its notes, or, where this
viewer has folded the branch, `⊞ n` plus the titles of what went, joined with
` · ` and clamped. That is the difference between a map of thirty labels and a map
of thirty thoughts, and between folding as hiding and folding as summarising. All
four readings are pure functions in `lib/mindmap-lens.ts` — first sentence, fold
summary, trust, and what cutting an edge would detach — because none of them can
be tested through a canvas jsdom cannot lay out. The fold summary is computed in
`Live` rather than in `Canvas`: the canvas is handed the VISIBLE nodes, and the
nodes a summary is about are exactly the ones missing from that list.

**The trust lens is a lens.** One toggle tints every node by how much anybody has
confirmed about it — a person wrote it and confirmed it, an agent wrote it and
nobody has checked, or nobody has confirmed it either way — from `origin` and
`reviewed`, which were already stored and already written by both the API and the
canvas. Nothing new is persisted. It is OFF by default and remembered per viewer
beside fold and zoom, because on a map an agent has been growing "what in here has
nobody looked at?" is a question you ask occasionally, not a decoration you live
with. The reading is never colour alone: the card carries a glyph with a title,
and a legend appears with the lens. Reachable from ⌘K, from a control on the
canvas, and — since a phone has neither — from a `md:hidden` button in the top
strip.

**A question is a node plus a relationship, and answering it is a person typing.**
`kind: 'question'` renders as its own shape with an "open question" eyebrow, and
what it questions is an ordinary relationship, drawn distinctly. So nothing in the
store had to learn what a question is, and an agent poses one with two calls it
already had. Its dialog offers an answer box; answering appends those words to
the notes of the node the question was about, marks that node reviewed, and
removes the question — an answered question is not an open question. A question
about nothing in particular keeps its own answer and stops being a question. There
is no model call in this path and none is wanted.

**Double-clicking empty canvas captures a loose thought** — a first-ring node
pinned where it was dropped, with the title caret already in it. You do not always
know where a thought goes, and forcing a parent is wrong for the ten minutes a
brainstorm is for.

**Clicking the line to a parent offers to cut it, behind two questions.** The
child becomes a first-ring thought and nothing is removed, but the gesture is one
click on a deliberately fat transparent target — so it goes through
`DetachDialog`, which always asks twice, the way `PruneDialog` asks twice for a
branch. Modelled on it rather than sharing it: that one asks twice only for a
branch. The detached node stays exactly where it was drawn rather than being sent
to the end of the ring, because somebody clicked a line and the map should not
jump under them.

Which commands ⌘K offers is a pure function (`lib/mindmap-commands.ts`), so it is
tested rather than reviewed: scope is the selected node else the map, and a
command that does not apply is ABSENT rather than disabled — on a read-only token
that would otherwise be most of the list. The same module owns the fuzzy match
behind "go to node…", which is how you reach a node off screen now that there is
no rail. The shortcut listener is registered in the CAPTURE phase. The pill, the right-click
menu and the title caret all stop every keydown — a button in a toolbar must not
fold a branch, and `Delete` typed into a name must not prune one — and React
attaches its handlers at the root container, so a synthetic stopPropagation there
stops the native event before it reaches the window. Capture is what keeps ⌘K out
from under all three, and it refuses to OPEN over a text field of its own accord.

**One app, one router, one bundle.** This replaced four independently-built
self-contained documents. That shape let the binary `include_str!` a whole page
and needed no asset routes at all — but React and every shared module were paid
four times, and moving between surfaces was a full page load that dropped all
warm state.

**Two collaborative routes, and one shared chunk under them.** `/documents` and
`/mindmaps` are both CRDTs over a Yjs `Y.Doc`, synchronised through the same
`tkd_` socket ticket and the same `y-websocket` provider — so both are
`lazy()`-imported in `src/App.tsx` rather than eagerly bundled. `vite.config.ts`
splits their dependencies in two: **collab** (`yjs`, `y-websocket`,
`y-protocols`, `lib0`, `isomorphic.js`) is a named chunk the two share, and
**editor** (`@tiptap`, `prosemirror-*` and friends) stays reachable only from
`/documents`. Neither goes to `vendor`, because vendor is preloaded by
`index.html` and would be paid on first paint by every surface. `collab` is
*named* rather than left to the bundler because TWO dynamic chunks import it and
an unnamed shared dependency is duplicated into both.

The mindmap model is split the same way for the same reason:
`src/lib/mindmap-doc.ts` holds the shape, the caps and the deterministic read
rules and imports nothing, while `src/lib/mindmap-crdt.ts` is the only file that
touches Yjs. A component can name a `MapNode` with a type-only import and pay
zero bytes for it, and the normalisation rules — cycle, orphan, dangling
relation, unusable order key — are tested without a document at all
(`mindmap-doc.test.ts`). Fold state is per-viewer and lives in `localStorage`
beside pan and zoom, never in the document: collapsing a branch must not collapse
it under somebody else mid-conversation.

**Tailwind scans `src/` and nothing else.** `globals.css` opens with
`@import 'tailwindcss' source(none)` plus two explicit `@source` lines, because
automatic content detection scans the project minus whatever is gitignored — and
`dist/` used to be committed here. Tailwind was therefore scanning the bundle it had just
written, which made the emitted CSS a fixed point of "sources + previous dist"
instead of a function of the sources: the first build after a merge was stale
(this failed CI's `dist is current` gate on two separate PRs, each time looking
like a flaky compiler), and a class could never leave the stylesheet once it had
shipped. `scripts/` and this README were being scanned too, so `h-screen`,
`w-72` and `max-w-140` — classes the lint rules exist to BAN — were compiled in
because the rule messages name them.

An allow list rather than an exclusion list, deliberately: an exclusion list has
to grow every time a script, test or doc happens to mention a class name.
Removing it all cut the stylesheet from 63.7 kB to 53.0 kB (11.8 → 10.4 kB gz).
`npm run size` carries three canaries against the regression, because nothing
else would notice — the build would stay self-consistent, just steadily larger.

**The asset names are load-bearing.** Rust embeds `assets/app.js`,
`assets/vendor.js`, `assets/runtime.js` and `assets/app.css` BY NAME, so content
hashing is off. Two consequences: cache correctness comes from an ETag rather
than the filename, and a fifth chunk would be referenced by `index.html` and
then 404 because nothing embeds it. `vite.config.ts` fails the build if the
output is not exactly that set — it caught the bundler's own runtime chunk the
first time it ran, which is why `runtime.js` is in the list at all (renamed from
the bundler's internal name so neither Rust nor the CSP encodes "rolldown").

**What the trade actually cost and bought.** Measured:

| | gzipped |
|---|---|
| before — one page | ~106 kB |
| before — all four surfaces | ~421 kB |
| **now — first load** | **~159 kB** |
| **now — every later route** | **0 kB** |

So first paint got *worse* for someone who opens one surface and leaves: `app.js`
carries all four surfaces, not one. It gets better the moment they navigate, and
much better across a session. `npm run size` guards first load (320 kB) and the
vendor chunk separately (180 kB) — vendor is where a careless `npm i` lands. Both
are a ceiling rather than a target: they are set well above today's ~197 kB so
ordinary work never trips them, which means what they still catch is a
heavyweight dependency. If first load creeps toward them through ordinary growth,
split the bundle per route rather than raising them again.

If first paint ever matters more than it does today, route-level splitting is the
lever: emit `assets/board.js` and friends, add them to `EMBEDDED` in
`vite.config.ts`, `include_str!` them, and serve them. The build guard makes that
a deliberate change rather than a silent one.

**TypeScript is pinned to 6.x on purpose.** 7.0 is the current stable release,
but `typescript-eslint` refuses it at runtime (upstream issue #10940) and its
supported range stops below 6.1. Losing `react-hooks/exhaustive-deps` — the rule
that catches stale closures in polling code — costs more than TS 7 buys. Revisit
when that lands.

**`dangerouslySetInnerHTML` and `innerHTML =` are eslint errors**, not
conventions. Every surface renders agent- and human-written text.
`src/lib/markdown.ts` builds DOM nodes; `<Markdown>` mounts them with
`replaceChildren`. That is the only sanctioned path, and it is why markup in a
ticket body cannot become markup in the page.

**Locale parity is a type, not a test.** `defineStrings({ en, de })` makes a
missing or extra DE key a compile error. The Rust-side
`spa_string_tables_agree_on_every_key` enumerated the four pages by hand and
could not see a new one; this cannot be forgotten.

## What is committed

Frontend source and the lockfile are committed. `dist/` and `dist-lib/` are
generated and gitignored; never force-add them. From a fresh checkout, use
`./scripts/build.sh` at the repository root (Node 22 and Rust required), or
`npm ci && npm run build` here before running Cargo.

CI builds the frontend once and passes its artifact to the Rust job. Docker uses
a Node build stage and copies its assets into the Rust stage. Render and Backlot
use the same build script. The final server binary still embeds all assets and
does not need Node or a separate frontend server at runtime.

Mermaid diagrams render in document code blocks whose language is `mermaid`,
including proposals, and in Markdown previews with fenced `mermaid` blocks.
In the document editor, type three backticks followed by `mermaid` and press
Enter to start one. The diagram appears above its editable source and updates
when typing pauses. Invalid diagrams keep their source and show an error.
The renderer loads only when a diagram is shown; diagrams use strict mode and
isolated SVG images, so Mermaid click actions are intentionally unavailable.
