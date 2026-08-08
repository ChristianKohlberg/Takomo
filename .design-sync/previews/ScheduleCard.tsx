import { ScheduleCard } from '@takomo/web'

const noop = () => {}
const ago = (ms: number) => new Date(Date.now() - ms).toISOString()
const L = {
  every: 'every', onDay: 'on day', day: 'daily', week: 'weekly', month: 'monthly',
  days: 'days', weeks: 'weeks', months: 'months',
  statusPending: 'waiting', statusActive: 'active', statusPaused: 'paused',
  statusRejected: 'rejected', statusRetired: 'retired',
  actActivate: 'Activate', actReject: 'Reject', actPause: 'Pause', actResume: 'Resume',
  actRun: 'Run now', actDelete: 'Delete',
  outDone: 'done', outOpen: 'open', outNf: 'not fulfilled',
  nextAt: 'next', noneScheduled: 'paused — nothing scheduled', proposedBy: 'proposed by',
  lastN: 'last', nowArrow: 'now →', neverFired: 'has never fired',
}
const occ = (i: number, outcome: 'done' | 'not_fulfilled' | null) => ({
  slot: new Date(Date.now() - i * 7 * 86_400_000).toISOString(),
  ticket: `demo-${1000 + i}`,
  title: `Weekly review — 2026-W${33 - i}`,
  outcome,
  claimed_by: outcome === 'done' ? 'agent:w1' : null,
})
const frame: React.CSSProperties = { maxWidth: 900 }

/** An active cadence with its history: the axis carrying all the meaning. */
export function Active() {
  return (
    <div style={frame}>
      <ScheduleCard
        schedule={{
          id: 'sch-1', project: 'demo', name: 'Weekly review', status: 'active',
          cadence: { every: 'week', on: ['mon'], at: '09:00', tz: 'Europe/Berlin' },
          next_slot: new Date(Date.now() + 2 * 86_400_000).toISOString(),
          occurrences: [occ(0, null), occ(1, 'done'), occ(2, 'not_fulfilled'), occ(3, 'done'), occ(4, 'done')],
        }}
        labels={L} lang="en" busy={false}
        onAction={noop} onRun={noop} onDelete={noop} onOpenTicket={noop}
      />
    </div>
  )
}

/**
 * A proposal shows the ticket it WOULD create — title and body. Approving a
 * cadence without seeing its ticket is approving a name.
 */
export function Proposal() {
  return (
    <div style={frame}>
      <ScheduleCard
        schedule={{
          id: 'sch-2', project: 'demo', name: 'Rotate the deploy key', status: 'pending',
          cadence: { every: 'month', day: 1, at: '09:00', tz: 'Europe/Berlin' },
          rationale: 'Rotated by hand three months running. Same work each time, so proposing a cadence.',
          proposed_by: 'agent:runner-1',
          template: { title: 'Rotate the deploy key — {month}', body: 'Rotate, then attach the **commit** that records the new fingerprint.' },
        }}
        labels={L} lang="en" busy={false}
        onAction={noop} onRun={noop} onDelete={noop} onOpenTicket={noop}
      />
    </div>
  )
}

/** Paused: dimmed, and honest that nothing is scheduled. */
export function Paused() {
  return (
    <div style={frame}>
      <ScheduleCard
        schedule={{
          id: 'sch-3', project: 'demo', name: 'Nightly backup check', status: 'paused',
          cadence: { every: 'day', at: '02:00', tz: 'UTC' },
          occurrences: [occ(1, 'done'), occ(2, 'done')],
        }}
        labels={L} lang="en" busy={false}
        onAction={noop} onRun={noop} onDelete={noop} onOpenTicket={noop}
      />
    </div>
  )
}
