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
//   * STABLE asset names. Rust `include_str!`s the asset paths by name, so
//     content hashes are turned off deliberately. Cache correctness is handled
//     by ETag revalidation in the server, not by the filename.
//   * EXACTLY the assets in EMBEDDED below. Any other chunk would be emitted,
//     referenced by index.html, and then 404 at runtime because nothing embeds
//     it. The build fails loudly rather than shipping that — which is not
//     hypothetical: it caught the bundler's runtime chunk on the first run.
//
// The library build lives in vite.lib.config.ts — same source, different
// consumer (see that file).
import { defineConfig, type Plugin } from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { readdirSync } from 'node:fs'
import { relative, resolve } from 'node:path'

/** The asset set the Rust binary embeds by name. Keep in sync with src/api/mod.rs. */
const EMBEDDED = [
  'index.html',
  'assets/app.js',
  'assets/vendor.js',
  'assets/runtime.js',
  'assets/app.css',
]

/**
 * Fail the build if the output is not exactly what the binary embeds.
 *
 * Without this, adding a dynamic `import()` silently emits a fourth chunk:
 * index.html references it, `cargo build` still succeeds because nothing changed
 * about the three files it names, and the app 404s in the browser on a route
 * nobody tested. Cheap check, whole class of bug.
 */
function assertEmbeddableOutput(): Plugin {
  return {
    name: 'takomo:assert-embeddable-output',
    apply: 'build',
    // `closeBundle`, not `generateBundle`: index.html is emitted by Vite's own
    // html plugin and is not in the bundle object yet when generateBundle runs,
    // so checking there reports it missing every time. Reading the finished
    // directory is also simply the more honest check — it is what the Rust build
    // will `include_str!`.
    closeBundle() {
      const outDir = resolve(import.meta.dirname, 'dist')
      const walk = (dir: string): string[] =>
        readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
          e.isDirectory() ? walk(resolve(dir, e.name)) : [relative(outDir, resolve(dir, e.name))],
        )
      const emitted = walk(outDir).sort()
      const unexpected = emitted.filter((f) => !EMBEDDED.includes(f))
      const missing = EMBEDDED.filter((f) => !emitted.includes(f))
      if (unexpected.length || missing.length) {
        this.error(
          `the build must emit exactly the files src/api/mod.rs embeds.\n` +
            `  expected: ${EMBEDDED.join(', ')}\n` +
            `  emitted:  ${emitted.join(', ')}\n` +
            (unexpected.length ? `  unexpected: ${unexpected.join(', ')}\n` : '') +
            (missing.length ? `  missing: ${missing.join(', ')}\n` : '') +
            `If this is intentional, add the file to include_str! in src/api/mod.rs, ` +
            `serve it from src/server.rs, and update EMBEDDED here.`,
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
        // One vendor chunk, shared by every route. This is the whole point of
        // the change: React was previously inlined into four documents.
        manualChunks: (id) => (id.includes('node_modules') ? 'vendor' : undefined),
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
    include: ['src/**/*.test.ts', 'src/**/*.test.tsx'],
  },
})
