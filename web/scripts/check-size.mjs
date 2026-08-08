// Per-page size budget.
//
// The four documents share nothing at runtime — no chunk, no cache — so every
// dependency is paid four times over. With plain React 19 (no Preact alias) the
// floor is already ~63 kB gz before any UI, which leaves little room: this gate
// is what turns "someone imported a date library" into a failed build instead
// of a page that quietly doubled.
//
// The budget is deliberately absolute, not a delta: a ratchet that only ever
// compares against the last build normalises the drift it is meant to catch.
import { readdir, readFile } from 'node:fs/promises'
import { gzipSync } from 'node:zlib'
import { resolve } from 'node:path'

// Measured floors, so the number is not folklore:
//
//   12.4 kB   schedules.html today (hand-written, no framework)
//   47.0 kB   board.html today, fully featured
//   60.9 kB   a placeholder page with React 19 and nothing else
//   91.2 kB   a placeholder page with React + four shadcn primitives
//
// 91 kB is therefore the FLOOR for any page here — before a single line of
// Takomo's own UI. The budget is not a target we are meeting; it is a tripwire
// for accidents (a stray heavy import, a second icon set), set high enough that
// a real port fits and low enough that doubling gets caught.
//
// If this fails on a legitimate port, raise it and record the new floor above.
// Do not delete the gate: four documents share nothing, so every dependency is
// paid four times and nothing else in the build will tell you.
const BUDGET_KB = 140

const dist = resolve(import.meta.dirname, '..', 'dist')
const files = (await readdir(dist)).filter((f) => f.endsWith('.html'))

if (files.length === 0) {
  console.error('size: no pages in dist/ — run `npm run build` first')
  process.exit(2)
}

let failed = false
for (const file of files.sort()) {
  const bytes = await readFile(resolve(dist, file))
  const gz = gzipSync(bytes).length / 1024
  const raw = bytes.length / 1024
  const over = gz > BUDGET_KB
  if (over) failed = true
  console.log(
    `${over ? 'FAIL' : 'ok  '} ${file.padEnd(20)} ${raw.toFixed(1).padStart(7)} kB raw   ${gz
      .toFixed(1)
      .padStart(6)} kB gz   (budget ${BUDGET_KB})`,
  )
}

if (failed) {
  console.error(`\nsize: a page exceeded ${BUDGET_KB} kB gzipped.`)
  console.error('Every page pays for every dependency separately — check what was imported.')
  process.exit(1)
}
