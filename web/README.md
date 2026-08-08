# web — Takomo's browser surfaces

One source, **two builds**, because two different consumers want different things:

| build | command | output | consumed by |
|---|---|---|---|
| **app** | `npm run build` | `dist/{board,inbox,initiatives,schedules}.html` — one self-contained document each | the Rust binary, via `include_str!` |
| **library** | `npm run build:lib` | `dist-lib/index.js` + `index.css` + `index.d.ts` | `claude.ai/design` (the design-sync skill) |

The app build is what ships. The library build exists because a design system is
consumed as components with a props contract, not as four inlined documents —
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
npm run size         # per-page gzip budget
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

## Decisions worth knowing before editing

**Four builds, not one with four entries.** Inlining everything requires
`output.codeSplitting: false`, and rollup refuses that with multiple inputs. So
`scripts/build-pages.mjs` drives four sequential builds over one shared
`vite.config.ts`. Each document therefore carries its own React — there is no
shared chunk and no cross-page caching, by construction.

**The size budget is real, and the floor is high.** Measured:

| | gzipped |
|---|---|
| `schedules.html` today (no framework) | 12.4 kB |
| `board.html` today, fully featured | 47.0 kB |
| placeholder + React 19 only | 60.9 kB |
| **placeholder + React + 4 shadcn primitives** | **91.2 kB** |

91 kB is the floor before a line of Takomo's own UI. `npm run size` fails past
140 kB per page — a tripwire for accidents, not a target we are meeting. Every
dependency is paid four times over, because the four documents share nothing.

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
