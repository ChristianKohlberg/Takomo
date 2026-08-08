import { TokenGate } from '@takomo/web'

const noop = () => {}

// `TokenGate` is `position: fixed; inset: 0` — it is a full-screen gate, so in a
// preview it escapes the card and gets clipped (the render check catches this as
// a 0px measured height). A `transform` on an ancestor makes it a containing
// block for fixed descendants, which is what gives the gate a real box here
// without changing the component.
const frame: React.CSSProperties = {
  position: 'relative',
  height: 520,
  width: '100%',
  maxWidth: 760,
  transform: 'translateZ(0)',
  overflow: 'hidden',
  border: '1px solid var(--border)',
  borderRadius: 12,
}

/**
 * The page itself is unauthenticated; every data fetch carries the bearer token
 * the viewer supplies here, kept in localStorage per origin. Serving the HTML
 * leaks nothing the API does not already guard.
 */
export function Empty() {
  return (
    <div style={frame}>
    <TokenGate
      title="takomo · initiatives"
      subtitle="A token with 'read' and 'write' — initiatives are read and fed here."
      tokenLabel="API token"
      openLabel="Open"
      emptyMessage="Enter a token."
      onSubmit={noop}
    />
    </div>
  )
}

/** A rejected token: the message is the API's, surfaced rather than reworded. */
export function Rejected() {
  return (
    <div style={frame}>
    <TokenGate
      title="takomo · initiatives"
      subtitle="A token with 'read' and 'write' — initiatives are read and fed here."
      tokenLabel="API token"
      openLabel="Open"
      initialToken="tk_live_a1b2c3d4"
      error="This token can only read. Writing needs the 'write' scope."
      emptyMessage="Enter a token."
      onSubmit={noop}
    />
    </div>
  )
}

/** German. */
export function German() {
  return (
    <div style={frame}>
    <TokenGate
      title="takomo · Initiativen"
      subtitle="Token mit 'read' und 'write' — Initiativen werden hier gelesen und gefüttert."
      tokenLabel="API-Token"
      openLabel="Öffnen"
      emptyMessage="Token eingeben."
      onSubmit={noop}
    />
    </div>
  )
}
