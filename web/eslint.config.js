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
      ],
    },
  },
]
