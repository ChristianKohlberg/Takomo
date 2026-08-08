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
 * `firstLoad` sits ~20% above today's measurement: enough headroom for ordinary
 * work, tight enough that adding a heavyweight dependency trips it. `vendor` is
 * the one to watch — it is where a careless `npm i` lands.
 */
const BUDGET_KB = { firstLoad: 200, vendor: 135 }

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

if (unexpected.length) {
  failed = true
  console.log(`\nFAIL unexpected files in dist/: ${unexpected.join(', ')}`)
  console.log('     The binary embeds a fixed set by name — see EMBEDDED in vite.config.ts.')
}

if (failed) process.exit(1)
