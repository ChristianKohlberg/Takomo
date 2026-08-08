import { Markdown } from '@takomo/web'

// The renderer parses agent- and human-written text. Every sample here is the
// kind of thing an agent actually writes into a ticket body or an initiative
// entry — which is also why the hostile-link case belongs in the card: refusing
// `javascript:` is a property of this component, not a footnote.

const wrap: React.CSSProperties = { maxWidth: 620 }

/** The common case: an agent's finding, with the structure it tends to use. */
export function AgentFinding() {
  return (
    <div style={wrap}>
      <Markdown
        text={[
          '`rollup_for_epic` counts **all** descendants, with no type filter.',
          '',
          'So a nested epic:',
          '',
          '- counts as one unit of work in its parent total',
          '- has its own work counted twice across two rows',
          '',
          '> The fix is ~20 lines plus an openapi block.',
        ].join('\n')}
      />
    </div>
  )
}

/** Tables render inside their own scroll container, so a wide one never widens the page. */
export function Table() {
  return (
    <div style={wrap}>
      <Markdown
        text={[
          '| case | today | wanted |',
          '|---|---|---|',
          '| sub-epic counted in total | yes | no |',
          '| work counted twice | yes | no |',
          '| percent diluted | yes | no |',
        ].join('\n')}
      />
    </div>
  )
}

/** Fenced code keeps its language and scrolls rather than reflowing. */
export function CodeBlock() {
  return (
    <div style={wrap}>
      <Markdown
        text={[
          'Point the detectors at the fork point instead:',
          '',
          '```sh',
          'HR_BASE=origin/main handrail run route-test-pairing openapi-current',
          '```',
        ].join('\n')}
      />
    </div>
  )
}

/** Headings start at h3 — these render inside panels that already own h1/h2. */
export function Headings() {
  return (
    <div style={wrap}>
      <Markdown
        text={[
          '# Concurrency is the load-bearing design',
          '',
          'Every mutation is one SQLite `IMMEDIATE` transaction.',
          '',
          '## Fencing',
          '',
          'A zombie worker writing with a stale fence gets a teaching 409.',
        ].join('\n')}
      />
    </div>
  )
}

/**
 * The link allowlist. Only http(s) and mailto become links; every other scheme
 * renders as its literal source, so a hostile link cannot hide behind link text.
 */
export function LinkSafety() {
  return (
    <div style={wrap}>
      <Markdown
        text={[
          'Allowed: [the spec](https://github.com/ChristianKohlberg/Takomo) and [mail](mailto:ada@example.com).',
          '',
          'Refused, printed verbatim: [click me](javascript:alert(1))',
        ].join('\n')}
      />
    </div>
  )
}
