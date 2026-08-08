# design-sync notes — Takomo

Repo-specific gotchas for future syncs. Read this before re-running anything.

## Shape and layout

- The design system is `web/`, **not** the repo root. The repo root is a Rust
  binary; `web/` is the frontend it embeds. Point `--node-modules` at
  `web/node_modules` and `--entry` at `web/dist-lib/index.js`.
- `web/` has **two** builds and only one of them is the DS:
  - `npm run build` → `web/dist/*.html`, four self-contained pages the Rust
    binary `include_str!`s. **Not** the design system.
  - `npm run build:lib` → `web/dist-lib/` — the component library, `.d.ts` tree,
    and stylesheet. **This** is what the sync consumes (`cfg.buildCmd`).
- There is no Storybook and there are no `*.stories.*` files — shape is
  `package`, confirmed by search, not assumed.

## Styling

- `dist-lib/index.css` only exists because `src/components/index.ts` imports
  `styles/globals.css`, **and** `vite.lib.config.ts` loads the Tailwind plugin.
  Remove either and the library ships JS with no CSS — components import fine and
  render as unstyled boxes, which no test catches. That is `cfg.cssEntry`.
- Colors are Takomo's own (the "Aquarelle" palette in `web/src/styles/tokens.css`).
  shadcn's tokens are **aliases** of them, mapped only inside `@theme inline` in
  `globals.css`. Three names collide with different meanings — `--accent`,
  `--muted`, `--border` — so re-running `shadcn init` re-appends a neutral grey
  palette at `:root` that silently overrides the brand. If previews come back
  grey, that is the cause.
- Dark mode is bound to `prefers-color-scheme`, not a `.dark` class. Screenshots
  render in whatever the headless browser reports (light by default).
- No web fonts ship: the stack is system UI + `ui-monospace`. `[FONT_MISSING]`
  should not fire; if it does, something re-added `@fontsource-variable/geist`,
  which was removed deliberately (228 kB × 4 inlined pages).

## Component set

- 35 PascalCase exports: 19 standalone components plus 16 compound sub-parts of
  `Card`/`Dialog` (`CardHeader`, `DialogTitle`, …). The sub-parts are real API and
  stay exported, but only make sense composed inside their parent — they are on
  the floor card by decision, not by failure.
- shadcn is copy-in, so `src/components/ui/*` is this repo's source, themed from
  `tokens.css`. It belongs in the DS: a design agent needs the same Button the app
  ships, not un-themed upstream defaults.

## Re-sync risks

- **The barrel is the contract.** A component added to `web/src/components/` but
  not exported from `src/components/index.ts` is invisible to the sync. Ports
  landed page by page and all four phases are now done (`/initiatives`,
  `/schedules`, `/inbox`, `/board`), taking the DS from 35 exports to 51.
- **Two-build trap.** `npm run build` alone does not refresh `dist-lib/`. Re-sync
  must run `npm run build:lib` (`cfg.buildCmd`) or it converts a stale library.
- **TypeScript is pinned to 6.x** because `typescript-eslint` refuses TS 7 at
  runtime (upstream #10940). If a future sync sees odd `.d.ts` extraction, check
  whether that pin moved.
- `dist-lib/` is gitignored and regenerated; `web/dist/` is committed. Do not
  confuse them.

## Fixes this run (first sync, 2026-08-08)

- **`[ZERO_MATCH]` on the first build.** `web/package.json` had no `types`/`main`,
  so the converter's `exportedNames()` looked for `web/index.d.ts` and found
  nothing — 0 components despite 30 parsed `.d.ts` files. Fixed properly by giving
  the package a conventional shape: `src/index.ts` as the public entry,
  `declarationDir: dist-lib` (so `dist-lib/index.d.ts` sits beside `index.js`),
  and `module`/`types`/`exports` in `package.json`. Do not remove those.
- **Tailwind preflight ate the markdown list bullets.** `ul { list-style: none }`
  from preflight silently stripped markers from every agent-written list. Restated
  in `markdown.css` — and note the shorthand does NOT survive: the minifier drops
  `list-style: disc outside` to `list-style: outside` because `disc` is the CSS
  initial value. The longhand `list-style-type: disc` does survive. This affected
  the live `/initiatives` page too, not just previews.
- **`TokenGate` measured 0px** (`[RENDER_THIN]`) because it is `position: fixed;
  inset: 0`. Its preview wraps it in a `transform: translateZ(0)` box, which makes
  a containing block for fixed descendants. Even so the render check flags
  `[GRID_OVERFLOW] (fixed/portal)`, so it is pinned `cardMode: single`.
- **Overlays** (`Dialog`, `CreateDialog`) render open and are pinned
  `cardMode: single` with an explicit viewport; `AppHeader` and `Composer` are
  `cardMode: column` (wider than a grid cell).

## Fixes this run (second sync, phases 2–4 ported)

- **`--entry` SKIPS `cfg.buildCmd`.** Passing `--entry ./web/dist-lib/index.js`
  (which this repo needs) makes `package-build.mjs` take the entry as given and
  never run `npm run build:lib`. Two full build → preview → capture → grade
  rounds were spent grading a STALE library before that surfaced. Always run
  `npm run build:lib` yourself first when passing `--entry`.
- **`Field` stretched every stacked form.** `Field` carried
  `flex-[1_1_170px]`, which reads as a WIDTH in the row layouts it was written
  for and as a HEIGHT in the stacked ones — so every field in `AskDrawer`,
  `SettingsSheet` and `CreateScheduleDialog` was padded to 170px tall and the
  footers were pushed off. The sizing now lives on the four row containers
  (`[&>*]:flex-[1_1_170px]`) and `Field` imposes none. This was a live defect on
  `/board` and `/schedules`, not a preview artifact — the review sheets found it.
- **`AnswerLinkDialog` printed a raw ISO timestamp.** The hand-written page ran
  it through `fmtWhen`; the port dropped that. It now takes a `lang` prop like
  every other date-rendering component.
- **The conventions header's negative examples went stale.** `text-crit`,
  `bg-high` and `text-low` did not exist at the first sync and DO now — the board
  and inbox cards use them to rank work. Telling the design agent to avoid a
  class that works is as harmful as the reverse. The header now carries
  re-verified examples on both sides plus a note to re-check rather than trust.
- **Full-height `position: fixed` drawers** (`DetailPanel`, `InboxDrawer`) need
  the same `transform: translateZ(0)` frame `TokenGate` uses, or they measure 0px
  and the sheet comes back blank.
- **Tall overlays need an explicit viewport** sized to the dialog, not wider: at
  `900x1280` the card scaled `SettingsSheet` down to unreadable. `640x900` shows
  it at a legible size. Overlays are all `cardMode: single`.

## Known render warns

None outstanding — the final validate exits clean with zero warnings.

`EditableText`'s `EditableTitle` and `ReadOnly` cells look identical in a static
capture. That is correct, not a defect: the edit affordance is a focus-state
dashed underline. If a future run flags `variants render identically` there, it is
this, and it is benign.

## Re-sync risks

- **Two builds, one of them easy to forget.** `npm run build` (the four pages) is
  NOT the design system. Re-sync must run `npm run build:lib` — that is `cfg.buildCmd`.
- **`shadcn init` re-appends its neutral palette** at `:root` in `globals.css`,
  which overrides `--accent`/`--muted`/`--border` and turns every preview grey.
  The reconciliation is documented at the top of that file; re-apply it if anyone
  re-runs init.
- **Utility classes are compiled from OUR source only.** The conventions header
  tells the design agent this, with verified examples on both sides
  (`bg-card` resolves; `text-crit`, `gap-7`, `p-9` do not). If components stop
  using a class, it silently leaves the stylesheet — re-verify the header's
  examples on any sync that changes styling.
- **The barrel is the contract.** `web/src/components/index.ts` decides what the
  DS contains. Phases 2–4 (schedules, inbox, board) will add components that must
  be exported there or they will not appear.
- Previews import from `'@takomo/web'`; that specifier is resolved by the
  converter, not by `node_modules` (the package does not self-install).
