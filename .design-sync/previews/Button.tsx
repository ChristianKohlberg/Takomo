import { Button } from '@takomo/web'

const row: React.CSSProperties = { display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap' }

/** The six variants, in the order a reader meets them on the surfaces. */
export function Variants() {
  return (
    <div style={row}>
      <Button>Append</Button>
      <Button variant="secondary">Load more</Button>
      <Button variant="outline">Cancel</Button>
      <Button variant="destructive">Withdraw question</Button>
      <Button variant="ghost">Dismiss</Button>
      <Button variant="link">docs/checklist.md</Button>
    </div>
  )
}

/** The size scale. `icon*` sizes are square and take a single glyph. */
export function Sizes() {
  return (
    <div style={row}>
      <Button size="lg">Create initiative</Button>
      <Button size="default">Append</Button>
      <Button size="sm">Load more</Button>
      <Button size="xs">edit</Button>
      <Button size="icon" variant="outline" title="Refresh">
        ↻
      </Button>
      <Button size="icon-sm" variant="outline" title="Sign out">
        ⎋
      </Button>
    </div>
  )
}

/** Disabled is the in-flight state: a write is running and must not double-fire. */
export function Disabled() {
  return (
    <div style={row}>
      <Button disabled>Appending …</Button>
      <Button variant="secondary" disabled>
        Load more
      </Button>
      <Button variant="outline" disabled>
        Cancel
      </Button>
    </div>
  )
}

/** How the header actually uses them: one primary, then icon affordances. */
export function HeaderActions() {
  return (
    <div style={row}>
      <Button>+ New initiative</Button>
      <Button variant="outline" size="icon" title="Refresh">
        ↻
      </Button>
      <Button variant="outline" size="icon" title="Sign out">
        ⎋
      </Button>
    </div>
  )
}
