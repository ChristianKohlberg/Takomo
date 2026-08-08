// The LIBRARY build: the same components, emitted as an importable ESM package
// with type declarations.
//
// Nothing in Takomo's own deployment consumes this — the binary serves the app
// build. This exists because a *design system* is consumed as components with a
// props contract, not as four inlined HTML documents: `claude.ai/design` (see
// the design-sync skill) reads `dist-lib/` to build on-brand screens out of
// Takomo's real parts.
//
// Keeping this build green is also a design constraint with teeth: a component
// that cannot be built here is one that reached into page-level state instead
// of taking props, which is exactly the coupling that makes a component
// unreusable. If `npm run build:lib` breaks, the fix belongs in the component.
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'
import { resolve } from 'node:path'

export default defineConfig({
  // Tailwind belongs in THIS build too, not just the app's. The barrel imports
  // globals.css, so the library emits a compiled stylesheet alongside the JS —
  // without it a consumer (the design system, above all) gets the components
  // with none of their styling, and every preview renders as unstyled boxes.
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { '@': resolve(import.meta.dirname, 'src') },
  },
  build: {
    outDir: 'dist-lib',
    emptyOutDir: true,
    lib: {
      entry: resolve(import.meta.dirname, 'src/index.ts'),
      formats: ['es'],
      fileName: 'index',
    },
    rollupOptions: {
      // React is the consumer's to provide; bundling it here would ship a
      // second copy into whatever host renders these components.
      external: ['react', 'react-dom', 'react/jsx-runtime'],
    },
  },
})
