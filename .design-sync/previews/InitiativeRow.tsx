import { InitiativeRow } from '@takomo/web'

const noop = () => {}
const ago = (ms: number) => new Date(Date.now() - ms).toISOString()

const wrap: React.CSSProperties = { maxWidth: 380, border: '1px solid var(--border)', borderRadius: 10, overflow: 'hidden' }

/** A list, which is the only way this component is ever seen. */
export function List() {
  return (
    <div style={wrap}>
      <InitiativeRow
        initiative={{
          id: 'ini-tm41jq69',
          project: 'takomo',
          title: 'Nested epics on the roadmap',
          summary: 'Whether /roadmap should present epic trees, and what it double-counts today.',
          status: 'parked',
          rollup: { entries: 3, attachments: 0, bytes: 586, last_entry_at: ago(23_000) },
        }}
        selected={false}
        statusLabel="Parked"
        entriesWord="entries"
        onSelect={noop}
      />
      <InitiativeRow
        initiative={{
          id: 'ini-8x2kd10p',
          project: 'takomo',
          title: 'Port the SPAs to a real frontend project',
          summary: 'Four hand-written pages, forked helpers, no tests. What a build step would buy.',
          status: 'open',
          rollup: { entries: 11, attachments: 2, bytes: 184_320, last_entry_at: ago(4 * 3600_000) },
        }}
        selected
        statusLabel="Open"
        entriesWord="entries"
        onSelect={noop}
      />
      <InitiativeRow
        initiative={{
          id: 'ini-q7m3ba55',
          project: 'takomo',
          title: 'Versioned migrations',
          summary: 'Probe-based migrate() cannot express a data migration.',
          status: 'distilled',
          rollup: { entries: 6, attachments: 0, bytes: 9_412, last_entry_at: ago(9 * 86_400_000) },
        }}
        selected={false}
        statusLabel="Distilled"
        entriesWord="entries"
        onSelect={noop}
      />
    </div>
  )
}

/** Selected: a tinted field plus an inset rule, never a left stripe. */
export function Selected() {
  return (
    <div style={wrap}>
      <InitiativeRow
        initiative={{
          id: 'ini-tm41jq69',
          project: 'takomo',
          title: 'Nested epics on the roadmap',
          summary: 'Whether /roadmap should present epic trees.',
          status: 'parked',
          rollup: { entries: 3, attachments: 1, bytes: 586, last_entry_at: ago(60_000) },
        }}
        selected
        statusLabel="Parked"
        entriesWord="entries"
        onSelect={noop}
      />
    </div>
  )
}

/** No summary and nothing accumulated yet — a title-only row still reads. */
export function Bare() {
  return (
    <div style={wrap}>
      <InitiativeRow
        initiative={{ id: 'ini-new00001', project: 'takomo', title: 'Postgres behind Store', status: 'open' }}
        selected={false}
        statusLabel="Open"
        entriesWord="entries"
        onSelect={noop}
      />
    </div>
  )
}
