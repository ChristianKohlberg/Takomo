import { Badge } from '@takomo/web'

const row: React.CSSProperties = { display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }

/** The variants. */
export function Variants() {
  return (
    <div style={row}>
      <Badge>default</Badge>
      <Badge variant="secondary">secondary</Badge>
      <Badge variant="outline">outline</Badge>
      <Badge variant="destructive">destructive</Badge>
    </div>
  )
}

/**
 * How Takomo actually uses it: one attribute encoded once. A tag is monospace
 * because it is an identifier; a label is not.
 */
export function TagsAndLabels() {
  return (
    <div style={row}>
      <Badge variant="secondary" className="font-mono">
        component:roadmap
      </Badge>
      <Badge variant="secondary" className="font-mono">
        person:ada
      </Badge>
      <Badge variant="outline">roadmap</Badge>
      <Badge variant="outline">needs-decision</Badge>
    </div>
  )
}

/** Entry kinds, the open vocabulary an initiative's collection is made of. */
export function EntryKinds() {
  return (
    <div style={row}>
      {['note', 'research', 'feedback', 'transcript', 'document', 'decision'].map((k) => (
        <Badge key={k} variant="secondary" className="uppercase">
          {k}
        </Badge>
      ))}
    </div>
  )
}
