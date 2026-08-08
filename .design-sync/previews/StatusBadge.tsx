import { StatusBadge } from '@takomo/web'

/**
 * An initiative's status is a LABEL, not a state machine — there is no workflow
 * behind it. The three tones carry that meaning: parked is a warning tone,
 * distilled the success tone, open the neutral chip.
 */
export function AllStatuses() {
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
      <StatusBadge status="open" label="Open" />
      <StatusBadge status="parked" label="Parked" />
      <StatusBadge status="distilled" label="Distilled" />
    </div>
  )
}

/** The component does no translation — the label is passed in, already localized. */
export function German() {
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
      <StatusBadge status="open" label="Offen" />
      <StatusBadge status="parked" label="Geparkt" />
      <StatusBadge status="distilled" label="Destilliert" />
    </div>
  )
}

/** In place: the badge sits beside a title and never wraps away from it. */
export function BesideATitle() {
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'baseline', maxWidth: 380 }}>
      <span style={{ fontWeight: 680, fontSize: 13.8 }}>Nested epics on the roadmap</span>
      <StatusBadge status="parked" label="Parked" />
    </div>
  )
}
