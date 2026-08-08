/// <reference types="vitest/config" />
//
// The APP build: four pages, each emitted as ONE self-contained document with
// every script, style and asset inlined.
//
// That single-document property is not a nicety — it is what lets the Rust
// binary keep `include_str!`ing each page (src/api/mod.rs), so Takomo stays one
// binary with no static-file handler, no second request, and no change to the
// `script-src 'self' 'unsafe-inline'` CSP the pages are already served under.
//
// Four separate entries rather than one client-side-routed app is also
// deliberate: every URL stays a real server-served page, so there is no history
// fallback to write and no chance of an unknown `/v1/...` path answering with
// HTML to an agent that expected a JSON 404.
//
// This file configures ONE page at a time. Inlining everything requires
// `output.codeSplitting: false`, and rollup refuses that with multiple inputs —
// so `npm run build` drives four builds over this config instead of one build
// with four entries (scripts/build-pages.mjs). Same shared config, four
// independent documents, which is what we wanted anyway.
//
// The library build lives in vite.lib.config.ts — same source, different
// consumer (see that file).
import { defineConfig } from 'vitest/config'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { viteSingleFile } from 'vite-plugin-singlefile'
import { resolve } from 'node:path'

export default defineConfig({
  plugins: [react(), tailwindcss(), viteSingleFile()],
  resolve: {
    // shadcn generates components that import from `@/lib/utils` and
    // `@/components/ui/*`; the alias is a hard requirement of that convention,
    // not a preference.
    alias: { '@': resolve(import.meta.dirname, 'src') },
  },
  build: {
    // Output lands in web/dist/ and is committed: that is what keeps
    // `cargo build --release` node-free on Render and in the Dockerfile.
    // Never emptied here — each page build would wipe the previous page's
    // output; the driver clears the directory once, up front.
    outDir: 'dist',
    emptyOutDir: false,
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
