// Defect-only ruleset, in the spirit of scripts/spa-eslint.config.mjs (which
// this eventually replaces): no style rules, so a red is always real.
//
// Two rules here are load-bearing rather than tidiness:
//
//   react/no-danger equivalent — `dangerouslySetInnerHTML` is BANNED. Every
//   surface renders agent- and human-written text, and the no-innerHTML-on-
//   user-data rule is the reason none of it can inject markup. src/lib/markdown
//   builds DOM nodes; that is the only sanctioned path.
//
//   react-hooks/exhaustive-deps is an ERROR, not a warning. Polling is the core
//   interaction on every page, and a stale closure in a poller is silent — it
//   shows up as "the board sometimes stops updating", which is unfalsifiable
//   from a bug report.
//
//   takomo/no-unresponsive-size bans a fixed dimension with no breakpoint
//   prefix. This is the rule that would have prevented essentially every mobile
//   defect an audit found: `grid-cols-[180px_320px_1fr]` on the inbox resolved
//   its third column to literally 0px on a phone, `w-72` columns made a 2400px
//   strip in a 375px window, `h-screen` put the bottom of every page under
//   mobile browser chrome, and `max-w-140` on a dialog silently deleted shadcn's
//   mobile inset. Every one of them is source that LOOKS correct — which is why
//   a lint rule beats a review.
import js from '@eslint/js'
import tseslint from 'typescript-eslint'
import reactHooks from 'eslint-plugin-react-hooks'

export default [
  { ignores: ['dist/**', 'dist-lib/**', 'node_modules/**'] },
  js.configs.recommended,
  {
    // Build scripts run in node, not the browser.
    files: ['scripts/**/*.mjs'],
    languageOptions: {
      globals: { process: 'readonly', console: 'readonly' },
    },
  },
  {
    files: ['src/**/*.{ts,tsx}'],
    plugins: { 'react-hooks': reactHooks },
    languageOptions: {
      // TypeScript is pinned to 6.x deliberately (package.json). 7.0 is the
      // current stable release, but typescript-eslint REFUSES it at runtime —
      // "typescript-eslint does not support TS 7.0", tracked upstream at
      // typescript-eslint#10940 — and its supported range stops at <6.1.0.
      // Rather than lose react-hooks/exhaustive-deps (the rule that catches the
      // stale-closure bugs polling code is prone to), the project sits on the
      // newest TS the linter accepts. Revisit when that issue closes.
      parser: tseslint.parser,
      parserOptions: { ecmaFeatures: { jsx: true } },
      ecmaVersion: 2022,
      sourceType: 'module',
      globals: { window: 'readonly', document: 'readonly', navigator: 'readonly', localStorage: 'readonly', fetch: 'readonly', console: 'readonly', setTimeout: 'readonly', clearTimeout: 'readonly', AbortController: 'readonly', performance: 'readonly' },
    },
    rules: {
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'error',
      'no-unused-vars': 'off', // tsc --noEmit owns this; eslint double-reports it
      'no-undef': 'off', // ditto — TS resolves types eslint cannot see
      'react/no-danger': 'off', // not loaded; the no-restricted-syntax below is the real guard
      'no-restricted-syntax': [
        'error',
        {
          selector: 'JSXAttribute[name.name="dangerouslySetInnerHTML"]',
          message:
            'dangerouslySetInnerHTML is banned. Render user/agent text through <Markdown>, which builds DOM nodes (src/lib/markdown.ts).',
        },
        {
          selector: 'AssignmentExpression[left.property.name="innerHTML"]',
          message:
            'Assigning innerHTML is banned on these surfaces. Build nodes instead — see src/lib/dom.ts.',
        },
        {
          // `h-screen` is `100vh`, which on mobile is the LARGE viewport — the
          // one with the URL bar retracted. Combined with the `overflow-hidden`
          // roots every page here uses, the bottom ~60-100px sits permanently
          // under browser chrome, which is exactly where the toast and the undo
          // snackbar live. `h-dvh` tracks the actual viewport.
          selector: 'Literal[value=/(^|\\s)h-screen(\\s|$)/]',
          message:
            'Use h-dvh, not h-screen: 100vh is the large viewport on mobile, so the bottom of the page sits under browser chrome.',
        },
        {
          // A fixed WIDTH over the narrowest supported viewport, with no
          // breakpoint prefix, does not fit on a phone. `max-w-*` is excluded on
          // purpose: it is a cap, so on a narrower screen it simply does not
          // bind and cannot overflow. Prefix it (`md:w-72`),
          // give it a mobile fallback (`w-[85vw] md:w-72`), or clamp it
          // (`max-w-[calc(100vw-2rem)]`).
          //
          // 320px is the threshold: the narrowest phone this targets is 320
          // wide, so anything at or below it fits by construction.
          selector:
            'Literal[value=/(^|\\s)(?<!(sm|md|lg|xl):)(w|min-w|basis)-\\[?(3[3-9][0-9]|[4-9][0-9]{2}|[0-9]{4,})px/]',
          message:
            'Fixed size over 320px with no breakpoint prefix will not fit a phone. Prefix it (md:w-96), give it a mobile fallback (w-[85vw] md:w-96), or clamp it (max-w-[calc(100vw-2rem)]).',
        },
        {
          // Tailwind's numeric scale is `n * 4px`, so `w-96` is 384px and
          // `max-w-140` is 560px — neither carries the string "px", so the rule
          // above cannot see them. 80 is the threshold: 80 * 4 = 320px, the
          // narrowest viewport this targets.
          selector:
            'Literal[value=/(^|\\s)(?<!(sm|md|lg|xl):)(w|min-w|basis)-(8[1-9]|9[0-9]|[1-9][0-9]{2,})(\\s|$)/]',
          message:
            'Fixed size over 320px (Tailwind scale is n*4px) with no breakpoint prefix will not fit a phone. Prefix it (md:w-96) or give a mobile fallback (w-full md:w-96).',
        },
        {
          // Multi-column fixed grids are the specific shape that produced the
          // worst defect found: two fixed columns totalling 500px left the third
          // resolving to 0px on every phone width, and an `overflow-hidden` root
          // meant it could not even be scrolled to.
          selector: 'Literal[value=/(^|\\s)(?<!(sm|md|lg|xl):)grid-cols-\\[[^\\]]*px[^\\]]*px/]',
          message:
            'A grid with two or more fixed px columns and no breakpoint prefix collapses the flexible column to 0px on a phone. Prefix it (md:grid-cols-[...]) and give mobile a single-column fallback.',
        },
      ],
    },
  },
]
