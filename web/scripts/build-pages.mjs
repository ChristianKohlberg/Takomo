// Build each page as its own self-contained document.
//
// Why a driver instead of `rollupOptions.input` with four entries: inlining
// every asset requires `output.codeSplitting: false`, and rollup rejects that
// as soon as there is more than one input —
//
//   [INVALID_OPTION] Invalid value "false" for option "output.codeSplitting"
//   - multiple inputs are not supported when "output.codeSplitting" is false
//
// so the four builds are sequential rather than one build with four entries.
// They share vite.config.ts; only the input and the log line differ.
//
// Cost of this shape: each document carries its own copy of React and of the
// shared lib, because there is no chunk for them to share. That is the price of
// the single-document property, and it is why package.json's `size` check
// exists — with plain React 19 (no Preact alias) the per-page floor is high
// enough that a careless import shows up immediately.
import { build } from 'vite'
import { rm, mkdir } from 'node:fs/promises'
import { resolve } from 'node:path'

const PAGES = ['board', 'inbox', 'initiatives', 'schedules']
const root = resolve(import.meta.dirname, '..')
const outDir = resolve(root, 'dist')

// One clean sweep up front — the per-page builds must not empty the directory
// or each would delete the last one's output.
await rm(outDir, { recursive: true, force: true })
await mkdir(outDir, { recursive: true })

for (const page of PAGES) {
  process.stdout.write(`\n— building ${page}.html\n`)
  await build({
    root,
    configFile: resolve(root, 'vite.config.ts'),
    build: {
      emptyOutDir: false,
      rollupOptions: { input: resolve(root, `${page}.html`) },
    },
  })
}
