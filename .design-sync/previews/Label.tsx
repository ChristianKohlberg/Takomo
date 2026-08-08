import { Input, Label } from '@takomo/web'

/**
 * Label is a leaf that only means anything attached to a control, so every cell
 * composes it with one — that is the only render that is true anyway.
 */
export function WithInput() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6, maxWidth: 380 }}>
      <Label htmlFor="src">Source</Label>
      <Input id="src" defaultValue="agent:w1" />
    </div>
  )
}

/** The form idiom these surfaces use: small uppercase caps above the field. */
export function FormCaps() {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14, maxWidth: 380 }}>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <Label
          htmlFor="kind"
          className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase"
        >
          Kind
        </Label>
        <Input id="kind" defaultValue="research" />
      </div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        <Label
          htmlFor="heading"
          className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase"
        >
          Heading
        </Label>
        <Input id="heading" placeholder="optional" />
      </div>
    </div>
  )
}
