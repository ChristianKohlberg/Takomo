import { Textarea } from '@takomo/web'

const stack: React.CSSProperties = { display: 'flex', flexDirection: 'column', gap: 10, maxWidth: 480 }

/** Empty with its placeholder, and holding a real markdown draft. */
export function States() {
  return (
    <div style={stack}>
      <Textarea className="min-h-24" placeholder="Markdown. A note, a finding, feedback …" />
      <Textarea
        className="min-h-24"
        defaultValue={
          '`rollup_for_epic` counts **all** descendants, with no type filter.\n\n- sub-epic counted in the parent total\n- its work counted twice'
        }
      />
    </div>
  )
}

/** Disabled — the in-flight state while an append is running. */
export function Disabled() {
  return (
    <div style={stack}>
      <Textarea className="min-h-20" defaultValue="Claimed the ticket and started the port." disabled />
    </div>
  )
}
