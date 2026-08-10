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

## Status: complete — all four surfaces are built from here

`src/api/mod.rs` serves all four pages from `web/dist/`. Every hand-written page is
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

## Commands

```sh
npm install
npm run dev          # vite on :5173, /v1 proxied — NO Rust rebuild in the loop
npm test             # vitest (113 tests)
npm run check        # tsc --noEmit
npm run lint         # eslint, defect rules only
npm run build        # the four pages
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

**One app, one router, one bundle.** This replaced four independently-built
self-contained documents. That shape let the binary `include_str!` a whole page
and needed no asset routes at all — but React and every shared module were paid
four times, and moving between surfaces was a full page load that dropped all
warm state.

**Tailwind scans `src/` and nothing else.** `globals.css` opens with
`@import 'tailwindcss' source(none)` plus two explicit `@source` lines, because
automatic content detection scans the project minus whatever is gitignored — and
`dist/` is COMMITTED here. Tailwind was therefore scanning the bundle it had just
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
much better across a session. `npm run size` guards first load (200 kB) and the
vendor chunk separately (135 kB) — vendor is where a careless `npm i` lands.

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

`dist/` **is** committed once pages are ported — that is what keeps
`cargo build --release` node-free on Render (`runtime: rust`) and in the
Dockerfile. Builds are byte-identical across rebuilds (verified), so a CI gate
can rebuild and diff. `dist-lib/` is not committed; it is regenerated on demand.
