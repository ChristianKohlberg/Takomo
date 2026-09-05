/// <reference types="vitest/config" />
//
// The APP build: ONE application, client-side routed, emitted as one HTML
// document plus a small set of shared assets.
//
// This replaced four independently-built self-contained documents. That earlier
// shape existed to let the Rust binary `include_str!` each page with no
// static-file handler at all, and it worked — but it meant React, the component
// library and every shared module were paid FOUR times, and moving between
// surfaces was a full page load that dropped all warm state.
//
// The binary still embeds everything: `include_str!` now covers index.html and
// three fixed asset paths instead of four documents (src/api/mod.rs). So Takomo
// remains ONE binary with nothing to deploy alongside it — which is the property
// render.yaml and the Dockerfile actually depend on — while the browser
// downloads the vendor chunk once and keeps it across all four routes.
//
// Two things this file must guarantee for the Rust side to compile at all:
//
//   * STABLE asset names. The server serves assets by name with content hashing
//     turned off deliberately; cache correctness is handled by ETag revalidation,
//     not by the filename.
//   * EVERYTHING FLAT UNDER assets/. `build.rs` embeds that one directory and the
//     server exposes it as `/assets/{file}`, so a nested directory would be
//     emitted, referenced by index.html, and then 404 at runtime.
//
// This used to be an exact-set assertion against four names, because Rust
// `include_str!`d each one. Code splitting ended that: the editor route is far
// too large for the single app.js chunk, and a dynamic `import()` emits a chunk
// whose name is not knowable when Rust is compiled. `build.rs` now generates the
// manifest from whatever is in dist/, so "a chunk nothing embeds" is impossible
// by construction rather than by a list two files had to keep in step.
//
// The library build lives in vite.lib.config.ts — same source, different
// consumer (see that file).
import { defineConfig, type Plugin } from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { readdirSync } from 'node:fs'
import { relative, resolve } from 'node:path'

/** What the server serves outside `/assets/{file}`. Keep in sync with src/api/mod.rs. */
const REQUIRED = ['index.html']

/**
 * The CRDT runtime, shared by the two lazily-loaded collaborative routes.
 *
 * This used to sit inside EDITOR_ONLY, back when `/documents` was the only thing
 * that synchronised anything. `/mindmaps` is now a second consumer, so leaving it
 * there would either duplicate Yjs into both lazy chunks or drag the editor's
 * ~150 kB of Tiptap and ProseMirror along behind a mindmap.
 *
 * It gets its OWN named chunk rather than going to vendor, and the distinction is
 * the whole point: vendor is preloaded by index.html, so anything in it is paid
 * on first paint by every surface. A named chunk reachable only through dynamic
 * imports is fetched when — and only when — one of those two routes opens.
 *
 * `lib0` is Yjs's own runtime and `isomorphic.js` is lib0's; neither is a
 * collaboration-shaped name, and leaving either out silently put it back on the
 * critical path while the build output still looked split.
 */
const COLLAB_PACKAGES = ['yjs', 'y-websocket', 'y-protocols', 'lib0', 'isomorphic\\.js']

/**
 * Packages reachable ONLY from the lazily-loaded `/documents` editor.
 *
 * Kept out of BOTH shared chunks — see `manualChunks` below for why that matters
 * more than it looks.
 *
 * This is the exact set the editor install added to the lockfile, minus the CRDT
 * runtime above, which is what makes it trustworthy rather than a guess: nothing
 * here existed before Tiptap did, so nothing else can need it. The transitive
 * names matter as much as the obvious ones — `linkifyjs` arrives via the Link
 * extension and `orderedmap` via ProseMirror's schema.
 *
 * A stray entry costs a slightly larger editor chunk. A missing one costs every
 * other surface, which is what `npm run size` is there to catch.
 */
const EDITOR_ONLY_PACKAGES = [
  '@tiptap',
  'prosemirror-[^/]+',
  'y-prosemirror',
  'linkifyjs',
  'fast-equals',
  'use-sync-external-store',
  'orderedmap',
  'rope-sequence',
  'w3c-keyname',
  'crelt',
]

const COLLAB = new RegExp(`node_modules/(${COLLAB_PACKAGES.join('|')})/`)
const EDITOR_ONLY = new RegExp(`node_modules/(${EDITOR_ONLY_PACKAGES.join('|')})/`)

/**
 * Fail the build if the output is not shaped the way the binary embeds it.
 *
 * Two things are still worth asserting now that the manifest is generated:
 * index.html must exist (Rust `include_str!`s it by name and the build would
 * fail confusingly without it), and every other emitted file must sit FLAT under
 * `assets/`. The route is `/assets/{file}` with a single path segment, so a
 * nested chunk would be referenced by index.html and 404 at runtime — the same
 * class of bug the old exact-set check caught, in the form that can still happen.
 */
function assertEmbeddableOutput(): Plugin {
  return {
    name: 'takomo:assert-embeddable-output',
    apply: 'build',
    // `closeBundle`, not `generateBundle`: index.html is emitted by Vite's own
    // html plugin and is not in the bundle object yet when generateBundle runs,
    // so checking there reports it missing every time. Reading the finished
    // directory is also simply the more honest check — it is what build.rs will
    // walk.
    closeBundle() {
      const outDir = resolve(import.meta.dirname, 'dist')
      const walk = (dir: string): string[] =>
        readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
          e.isDirectory() ? walk(resolve(dir, e.name)) : [relative(outDir, resolve(dir, e.name))],
        )
      const emitted = walk(outDir).sort()
      const missing = REQUIRED.filter((f) => !emitted.includes(f))
      // Exactly one segment below `assets/`, matching the `/assets/{file}` route.
      const misplaced = emitted.filter(
        (f) => !REQUIRED.includes(f) && !/^assets\/[^/]+$/.test(f),
      )
      if (missing.length || misplaced.length) {
        this.error(
          `the build must emit index.html plus a flat assets/ directory.\n` +
            `  emitted: ${emitted.join(', ')}\n` +
            (missing.length ? `  missing: ${missing.join(', ')}\n` : '') +
            (misplaced.length ? `  not flat under assets/: ${misplaced.join(', ')}\n` : '') +
            `build.rs embeds assets/ and src/server.rs serves it as /assets/{file}, ` +
            `which is one path segment.`,
        )
      }
    },
  }
}

export default defineConfig({
  plugins: [react(), tailwindcss(), assertEmbeddableOutput()],
  resolve: {
    // shadcn generates components that import from `@/lib/utils` and
    // `@/components/ui/*`; the alias is a hard requirement of that convention,
    // not a preference.
    alias: { '@': resolve(import.meta.dirname, 'src') },
  },
  build: {
    // Output lands in web/dist/ and is committed: that is what keeps
    // `cargo build --release` node-free on Render and in the Dockerfile.
    outDir: 'dist',
    emptyOutDir: true,
    rollupOptions: {
      output: {
        // No content hashes — see the header. The server sends an ETag instead.
        entryFileNames: 'assets/app.js',
        // The bundler emits its own small runtime as a separate chunk and does
        // NOT route it through manualChunks, so it cannot be folded into vendor.
        // It is renamed here instead: the served path stays `assets/runtime.js`
        // whatever the bundler calls it internally, so neither Rust nor the CSP
        // ends up encoding "rolldown".
        chunkFileNames: (info) =>
          info.name.includes('runtime') ? 'assets/runtime.js' : 'assets/[name].js',
        assetFileNames: (info) =>
          info.names?.some((n) => n.endsWith('.css')) ? 'assets/app.css' : 'assets/[name][extname]',
        // One vendor chunk for what every route shares. This is the whole point
        // of the earlier change: React was previously inlined into four
        // documents.
        //
        // EDITOR_ONLY and COLLAB are both excluded from it, and that exclusion
        // is the difference between a route being split and merely appearing to
        // be. Tiptap and ProseMirror are ~100 kB gz and reachable only from the
        // lazy `/documents` route; Yjs and its socket are ~50 kB and reachable
        // only from `/documents` and `/mindmaps` — but a blanket "everything in
        // node_modules goes to vendor" would sweep them into the chunk
        // index.html loads eagerly, so every other surface would pay for them on
        // first paint while the build output still showed a neat little
        // Editor.js. Returning undefined lets the bundler attach the editor's
        // packages to the dynamic chunk that imports them; `collab` is named
        // instead because TWO dynamic chunks import it, and an unnamed shared
        // dependency would be duplicated into both.
        manualChunks: (id) => {
          if (!id.includes('node_modules')) return undefined
          if (EDITOR_ONLY.test(id)) return undefined
          if (COLLAB.test(id)) return 'collab'
          return 'vendor'
        },
      },
    },
  },
  server: {
    // `vite dev` talks to a real backlot instance, so the dev loop never pays a
    // Rust rebuild. Point TAKOMO_DEV_API at whatever `backlot up` printed.
    proxy: {
      '/v1': { target: process.env.TAKOMO_DEV_API ?? 'http://127.0.0.1:8080', changeOrigin: true },
      '/mcp': { target: process.env.TAKOMO_DEV_API ?? 'http://127.0.0.1:8080', changeOrigin: true },
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    // Stubs for the browser APIs Radix's overlay primitives measure with; see
    // src/test-setup.ts for why a listbox cannot be opened in jsdom without them.
    setupFiles: ['./src/test-setup.ts'],
    include: ['src/**/*.test.ts', 'src/**/*.test.tsx'],
  },
})
