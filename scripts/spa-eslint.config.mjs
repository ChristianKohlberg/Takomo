// ESLint configuration for the two single-file SPAs (src/board.html,
// src/inbox.html). Driven by scripts/lint-spa.sh — never by a bare `eslint .`,
// since the JavaScript it lints lives inside HTML and is fed in on stdin.
//
// Deliberately a SMALL ruleset. The SPAs are hand-written, dependency-free ES5
// in a house style that no shared config agrees with, and a style-opinionated
// preset would bury the defects under hundreds of formatting complaints until
// somebody switched the job off. Every rule here is one that flags code which is
// wrong or dead, never code that is merely unfashionable — so a red run always
// means there is something to fix.
//
// The rule that motivated the lane: `no-dupe-keys`. src/board.html declared
// `ticketFilter` twice in the same object literal (takomo-rrjg). That is legal
// JavaScript — the last one wins, no error at parse time, no error at runtime —
// so the parse check agents keep hand-rolling would never have seen it.

// Web-platform globals the SPAs run against. Written out rather than pulled from
// the `globals` package: this lane's whole appeal is that it needs no dependency
// beyond eslint itself, and the list moves about once a year. A genuine browser
// global missing from this list shows up as a clear `no-undef` naming it — add
// it here. That is the cost of `no-undef` catching `docuemnt.getElementById`.
const BROWSER = [
  "document", "window", "location", "history", "navigator", "screen",
  "localStorage", "sessionStorage", "console", "alert", "confirm", "prompt",
  "fetch", "Headers", "Request", "Response", "AbortController", "XMLHttpRequest",
  "URL", "URLSearchParams", "Blob", "File", "FileReader", "FormData",
  "setTimeout", "clearTimeout", "setInterval", "clearInterval",
  "requestAnimationFrame", "cancelAnimationFrame", "queueMicrotask",
  "EventSource", "WebSocket", "Event", "CustomEvent", "MessageEvent",
  "MutationObserver", "ResizeObserver", "IntersectionObserver",
  "getComputedStyle", "matchMedia", "getSelection", "scrollTo",
  "Element", "HTMLElement", "Node", "NodeList", "DOMParser", "CSS",
  "crypto", "performance", "structuredClone", "btoa", "atob",
  "Intl", "TextEncoder", "TextDecoder",
];

export default [
  {
    // `**/*.html` is here because scripts/lint-spa.sh pipes the extracted
    // script through `--stdin-filename src/board.html`, so that findings are
    // reported at the real path and the real line number of the HTML file.
    files: ["**/*.js", "**/*.html"],
    languageOptions: {
      ecmaVersion: 2022,
      // The SPAs are plain classic scripts in a browser <script> tag: no
      // modules, no bundler, no build step. Saying so is what makes top-level
      // `var` and function declarations resolve the way they actually do.
      sourceType: "script",
      globals: Object.fromEntries(BROWSER.map((g) => [g, "readonly"])),
    },
    linterOptions: { reportUnusedDisableDirectives: "error" },
    rules: {
      // Duplicates and shadowing — silently discarded code.
      "no-dupe-keys": "error",
      "no-dupe-args": "error",
      "no-dupe-else-if": "error",
      "no-duplicate-case": "error",
      "no-dupe-class-members": "error",

      // Names that do not resolve, and code that nothing reaches.
      "no-undef": "error",
      "no-unreachable": "error",
      // `args: none` on purpose: these SPAs use fixed-arity DOM callbacks where
      // an ignored trailing parameter is documentation, not dead weight. Unused
      // *bindings* are still errors — that is what found the leftover
      // `parseShareToken` after answer-link mode generalised it.
      "no-unused-vars": ["error", { args: "none", caughtErrors: "none" }],

      // Assignments that cannot mean what they look like.
      "no-const-assign": "error",
      "no-func-assign": "error",
      "no-self-assign": "error",
      "no-self-compare": "error",
      "no-cond-assign": ["error", "except-parens"],

      // Comparisons and control flow that quietly do nothing.
      "use-isnan": "error",
      "valid-typeof": "error",
      "no-unsafe-negation": "error",
      "no-fallthrough": "error",
      "no-sparse-arrays": "error",
      "no-empty-pattern": "error",
      "no-compare-neg-zero": "error",

      // Deliberately NOT here: a ban on `innerHTML`, even though CLAUDE.md makes
      // "never innerHTML on user data" a house rule. The rule eslint can express
      // (`no-restricted-properties`) cannot tell user data from a constant, and
      // the SPAs hold ~37 innerHTML sites that are all either `= ""` container
      // clears or inline literal SVG — every one safe. Enforcing it would mean
      // 37 disable comments, which trains people to write disables rather than
      // to think. The norm needs a check that reads the right-hand side; that is
      // a different ticket, not a rule toggle here.
    },
  },
  {
    // src/spa-common.js (takomo-ftix) is inlined into both pages at build time,
    // and depends on exactly one thing from its host: `el(tag, cls, text)`. When
    // linted as a file in its own right that name is unresolvable, so declare it
    // here rather than with a `/* global */` directive in the source — the
    // directive would collide once the file is spliced into a page that declares
    // `el` for real, and `reportUnusedDisableDirectives` would then flag it.
    //
    // Deliberately narrow: one name, one file. If this list grows, the module has
    // grown a dependency on its host and stopped being shareable — which is the
    // signal to push back, not to extend this array.
    files: ["**/spa-common.js"],
    languageOptions: { globals: { el: "readonly" } },
    rules: {
      // Off for this file ONLY, and nothing is lost. In isolation every entry
      // point the module exists to provide is unused by definition — `mdNode`,
      // `mdInline` and the rest are called by the host pages, never from here.
      //
      // Dead code inside the module is still caught, and caught better: the page
      // lint splices this file in at the marker and lints the assembly, which is
      // the only context it ever runs in. A helper in here that nothing calls is
      // unused *there* too, and that run is the truthful one. So the standalone
      // pass is for the checks that need no host — parse errors, undefined names,
      // duplicate keys — and the spliced pass is for reachability.
      "no-unused-vars": "off",
    },
  },
];
