import { OccurrenceStrip } from '@takomo/web'

const noop = () => {}
const LABELS = { done: 'done', open: 'open', notFulfilled: 'not fulfilled', nowArrow: 'now →' }
const slot = (weeksAgo: number) => new Date(Date.now() - weeksAgo * 7 * 86_400_000).toISOString()

const occ = (i: number, outcome: 'done' | 'not_fulfilled' | null) => ({
  slot: slot(i),
  ticket: `demo-${['zq7t', 'bnzg', 'guxd', 'gmk6', 'doyw', 'ochd', 'uxpa', '5zk6'][i] ?? 'xxxx'}`,
  title: `Weekly review — 2026-W${33 - i}`,
  outcome,
  claimed_by: outcome === 'done' ? 'agent:w1' : null,
})

/**
 * Right-justified, newest LAST. A schedule with fewer than eight occurrences
 * still puts its newest cell flush right, because "now" is where the eye ends
 * up — in a grid the short row left a hole and the most recent one drifted off.
 */
export function FullHistory() {
  return (
    <div style={{ width: 720 }}>
      <OccurrenceStrip
        occurrences={[occ(0, null), occ(1, 'done'), occ(2, 'not_fulfilled'), occ(3, 'done'), occ(4, 'not_fulfilled'), occ(5, 'done'), occ(6, 'not_fulfilled'), occ(7, 'done')]}
        unit="week"
        lang="en"
        labels={LABELS}
        onOpenTicket={noop}
      />
    </div>
  )
}

/** Three occurrences, still flush right — the point of the layout. */
export function ShortHistory() {
  return (
    <div style={{ width: 720 }}>
      <OccurrenceStrip
        occurrences={[occ(0, null), occ(1, 'done'), occ(2, 'done')]}
        unit="week"
        lang="en"
        labels={LABELS}
        onOpenTicket={noop}
      />
    </div>
  )
}

// A daily schedule's slots are a DAY apart, so it needs its own fixture: reusing
// the weekly one would label the cells as dates while spacing them a week apart,
// which is the one thing this story exists to show.
const dayOcc = (i: number, outcome: 'done' | 'not_fulfilled' | null) => ({
  ...occ(i, outcome),
  slot: new Date(Date.now() - i * 86_400_000).toISOString(),
  title: 'Nightly backup check',
})

/** A daily cadence labels its cells as dates rather than ISO weeks. */
export function DailyCadence() {
  return (
    <div style={{ width: 720 }}>
      <OccurrenceStrip
        occurrences={[dayOcc(0, null), dayOcc(1, 'done'), dayOcc(2, 'done'), dayOcc(3, 'not_fulfilled')]}
        unit="day"
        lang="en"
        labels={LABELS}
        onOpenTicket={noop}
      />
    </div>
  )
}
