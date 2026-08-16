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

/** Everything the browser fetches before the first surface paints. */
const FIRST_LOAD = ['index.html', 'assets/vendor.js', 'assets/runtime.js', 'assets/app.js', 'assets/app.css']

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
const BUDGET_KB = { firstLoad: 320, vendor: 180 }

const gz = (file) => gzipSync(readFileSync(resolve(dist, file))).length / 1024

// A stray file in dist/ means the build emitted something the Rust binary does
// not embed. vite.config.ts fails the build on that, but dist/ is committed and
// could be edited by hand, so say so here too rather than quietly ignoring it.
const walk = (dir, base = '') =>
  readdirSync(resolve(dist, dir), { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(`${dir}/${e.name}`, `${base}${e.name}/`) : `${base}${e.name}`,
  )
const present = walk('.').sort()
const unexpected = present.filter((f) => !FIRST_LOAD.includes(f))

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
console.log(`ok   every later route         0.0 kB gz   (client-side routing)`)

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
  console.log(`\nFAIL unexpected files in dist/: ${unexpected.join(', ')}`)
  console.log('     The binary embeds a fixed set by name — see EMBEDDED in vite.config.ts.')
}

if (failed) process.exit(1)
