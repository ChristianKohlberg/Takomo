// Gzip budget for the app.
//
// The shape of this check changed with the move to one bundle. It used to be a
// per-page budget, because the four documents shared nothing and every
// dependency was paid four times — so the interesting number was "how big is
// ONE page". Now the interesting numbers are different:
//
//   * FIRST LOAD — everything the browser must fetch before the first surface
//     renders. This got WORSE with one app (~106 kB gz for a single page before,
//     ~164 kB now) because app.js carries all four surfaces, not one. That is
//     the honest cost of the trade.
//   * EVERY LATER ROUTE — zero. Moving to /inbox fetches nothing. Visiting all
//     four surfaces went from ~421 kB to ~164 kB, and navigation is instant.
//
// So the budget guards first load, and separately guards vendor, because vendor
// is the part that grows silently when someone adds a dependency.
import { readFileSync, readdirSync } from 'node:fs'
import { gzipSync } from 'node:zlib'
import { resolve } from 'node:path'

const dist = resolve(import.meta.dirname, '..', 'dist')

/**
 * Everything the browser fetches before the first surface paints — DERIVED from
 * index.html rather than listed here.
 *
 * It was a hardcoded list until routes started being code-split. A fixed list
 * gets this exactly backwards once chunks exist: the moment a route is lazily
 * loaded, the list still names it and the budget keeps charging for bytes the
 * first paint no longer waits on — so the check would report a regression for
 * the very change that fixed one. Reading the document's own references measures
 * what a browser actually blocks on.
 *
 * `modulepreload` links ARE counted, and that is the subtle part. They look
 * skippable — a hint, not a fetch — but Vite emits them for the entry's STATIC
 * dependency chunks, which the browser must have in hand before the entry can
 * execute. `vendor.js` arrives that way. Excluding them would silently drop the
 * largest thing on the critical path out of the budget.
 *
 * A dynamically imported chunk never appears in this document at all: Vite
 * preloads those at runtime through `__vitePreload`. So "what index.html
 * references" is exactly "what blocks the first paint", with no filtering needed.
 */
const firstLoadFiles = () => {
  const html = readFileSync(resolve(dist, 'index.html'), 'utf8')
  const refs = [...html.matchAll(/(?:src|href)="\/assets\/([^"]+)"/g)].map((m) => `assets/${m[1]}`)
  return ['index.html', ...new Set(refs)]
}
const FIRST_LOAD = firstLoadFiles()

/**
 * Budgets in gzipped kB.
 *
 * These are deliberately roomy — roughly 60% above today's measurement rather
 * than the ~20% they started at. The tight version had stopped doing its job:
 * first load reached 197.5 against a budget of 200, so the next feature of any
 * size would have tripped it, and a budget that fails on ordinary work gets
 * raised in the same commit as the work. A limit nobody can ship past is a limit
 * that gets edited, and an edited limit measures nothing.
 *
 * What they still catch, which is the point: a heavyweight dependency. `vendor`
 * is the one to watch — it is where a careless `npm i` lands, and 180 leaves
 * room for a genuinely useful library while a rich-text editor or a charting
 * suite would still trip it.
 *
 * These are a ceiling, not a target. If first load creeps toward them through
 * ordinary growth rather than one obvious addition, the answer is to split the
 * bundle per route — the router already makes that possible — not to raise these
 * again. Raising them a second time would be the moment this check became
 * decorative.
 */
const BUDGET_KB = {
  firstLoad: 320,
  vendor: 180,
  // The largest single chunk a route may pull on top of first load. Roomy in the
  // same spirit as the others — the point is to notice a step change, not to
  // police every kilobyte.
  lazyChunk: 220,
}

const gz = (file) => gzipSync(readFileSync(resolve(dist, file))).length / 1024

// A misplaced file in dist/ means the build emitted something the server cannot
// serve: build.rs embeds assets/ flat and the route is `/assets/{file}`, one path
// segment. vite.config.ts fails the build on that, but dist/ is committed and
// could be edited by hand, so say so here too rather than quietly ignoring it.
//
// This no longer checks against FIRST_LOAD. It used to, back when those two sets
// were the same thing — with code splitting they are not, and a lazy chunk is
// exactly the file that is legitimately present and legitimately not fetched
// first.
const walk = (dir, base = '') =>
  readdirSync(resolve(dist, dir), { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(`${dir}/${e.name}`, `${base}${e.name}/`) : `${base}${e.name}`,
  )
const present = walk('.').sort()
const unexpected = present.filter((f) => f !== 'index.html' && !/^assets\/[^/]+$/.test(f))

let failed = false

const firstLoad = FIRST_LOAD.reduce((sum, f) => sum + gz(f), 0)
for (const f of FIRST_LOAD) {
  console.log(`     ${f.padEnd(22)} ${gz(f).toFixed(1).padStart(7)} kB gz`)
}
console.log(`  ${'─'.repeat(44)}`)

const line = (label, value, budget) => {
  const ok = value <= budget
  if (!ok) failed = true
  console.log(
    `${ok ? 'ok  ' : 'FAIL'} ${label.padEnd(22)} ${value.toFixed(1).padStart(7)} kB gz   (budget ${budget})`,
  )
}

line('first load', firstLoad, BUDGET_KB.firstLoad)
line('└─ of which vendor', gz('assets/vendor.js'), BUDGET_KB.vendor)
// What a lazy route costs on top of first load.
//
// This line used to be a hardcoded "0.0 kB — client-side routing", printed with
// nothing behind it. That was TRUE when every route was eager and moving between
// them fetched nothing. Two routes are lazy now, and `/documents` pulls its own
// chunk plus the CRDT runtime — more than vendor.js — while this line went on
// reporting zero. A guard that prints a number it did not measure is worse than
// no guard: it reads as coverage.
const lazy = present.filter(
  (f) => f.startsWith('assets/') && f.endsWith('.js') && !FIRST_LOAD.includes(f),
)
const heaviest = lazy.reduce((worst, f) => (gz(f) > gz(worst) ? f : worst), lazy[0] ?? 'assets/app.js')
for (const f of lazy) {
  console.log(`     ${f.padEnd(22)} ${gz(f).toFixed(1).padStart(7)} kB gz   (lazy)`)
}
line('heaviest lazy chunk', gz(heaviest), BUDGET_KB.lazyChunk)

// The stylesheet must not contain utilities that exist only inside Tailwind's
// own output.
//
// `dist/` is committed, so it is not gitignored, and Tailwind's automatic
// content detection used to scan the bundle it had just written — finding the
// utility names in there and keeping them "in use" forever. `@source not` in
// globals.css stops that (see the note there). Nothing else would notice if
// that line were deleted: the build would stay self-consistent, just with a
// steadily growing stylesheet, so "dist is current" would still pass.
//
// These three are canaries, not a whitelist. Each is a real Tailwind utility
// that no Takomo source uses and that only ever appeared via the feedback loop.
// If one is back, so is the loop.
const CANARIES = ['.table-column-group{', '.oldstyle-nums{', '.zoom-in{']
const css = readFileSync(resolve(dist, 'assets/app.css'), 'utf8')
const leaked = CANARIES.filter((c) => css.includes(c))
if (leaked.length) {
  failed = true
  console.log(`\nFAIL app.css contains build-output-only utilities: ${leaked.join(' ')}`)
  console.log("     Tailwind is scanning dist/ again — check `@source not` in src/styles/globals.css.")
}

if (unexpected.length) {
  failed = true
  console.log(`\nFAIL files dist/ cannot serve: ${unexpected.join(', ')}`)
  console.log('     build.rs embeds assets/ flat; the route is /assets/{file}, one segment.')
}

if (failed) process.exit(1)
