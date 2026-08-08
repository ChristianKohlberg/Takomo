import { Input } from '@takomo/web'

const stack: React.CSSProperties = { display: 'flex', flexDirection: 'column', gap: 10, maxWidth: 380 }

/** Empty, filled, and disabled — the three states a form field is ever in. */
export function States() {
  return (
    <div style={stack}>
      <Input placeholder="What is the idea called?" />
      <Input defaultValue="Nested epics on the roadmap" />
      <Input defaultValue="ini-tm41jq69" disabled />
    </div>
  )
}

/** The types the surfaces actually use. */
export function Types() {
  return (
    <div style={stack}>
      <Input type="search" placeholder="Search title and summary" />
      <Input type="password" defaultValue="tk_live_secret" className="font-mono" />
      <Input type="datetime-local" />
      <Input type="file" />
    </div>
  )
}

/**
 * Monospace for identifiers. A token or a ticket id is an identifier, and
 * reading it as one is the point.
 */
export function Monospace() {
  return (
    <div style={stack}>
      <Input className="font-mono" defaultValue="tk_live_a1b2c3d4e5f6" />
      <Input className="font-mono" defaultValue="agent:w1" />
    </div>
  )
}
