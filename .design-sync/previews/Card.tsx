import { Badge, Button, Card, CardAction, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@takomo/web'

// Card is a compound: the parts only mean anything assembled, so every cell here
// is a whole card rather than a bare part.

const wrap: React.CSSProperties = { maxWidth: 460 }

/** The full anatomy — header, action, description, content, footer. */
export function Anatomy() {
  return (
    <div style={wrap}>
      <Card>
        <CardHeader>
          <CardTitle>Nested epics on the roadmap</CardTitle>
          <CardDescription>
            Whether /roadmap should present epic trees, and what it double-counts today.
          </CardDescription>
          <CardAction>
            <Badge variant="secondary">parked</Badge>
          </CardAction>
        </CardHeader>
        <CardContent>
          <p style={{ margin: 0 }}>
            The rollup counts every descendant with no type filter, so a sub-epic is both a work
            item in its parent and a row of its own.
          </p>
        </CardContent>
        <CardFooter style={{ gap: 8 }}>
          <Button size="sm">Open</Button>
          <Button size="sm" variant="outline">
            Dismiss
          </Button>
        </CardFooter>
      </Card>
    </div>
  )
}

/** Header + content only: the shape most surfaces actually use. */
export function Minimal() {
  return (
    <div style={wrap}>
      <Card>
        <CardHeader>
          <CardTitle>Add an entry</CardTitle>
        </CardHeader>
        <CardContent>
          <p style={{ margin: 0 }}>
            Entries are append-only. Each one records where it came from, which is what makes the
            collection weighable later.
          </p>
        </CardContent>
      </Card>
    </div>
  )
}

/** A metric row, the way the initiative rollup presents accumulated work. */
export function Metrics() {
  const cells: [string, string][] = [
    ['Entries', '3'],
    ['Attachments', '0'],
    ['Characters', '586'],
    ['Size', '586 B'],
  ]
  return (
    <div style={{ maxWidth: 560 }}>
      <Card>
        <CardContent style={{ display: 'flex', gap: 0, flexWrap: 'wrap', paddingTop: 20 }}>
          {cells.map(([k, v]) => (
            <div key={k} style={{ flex: '1 1 110px' }}>
              <div
                style={{
                  fontSize: 10.5,
                  fontWeight: 700,
                  letterSpacing: '.05em',
                  textTransform: 'uppercase',
                  color: 'var(--muted)',
                }}
              >
                {k}
              </div>
              <div style={{ fontSize: 17, fontWeight: 720, marginTop: 2 }}>{v}</div>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  )
}
