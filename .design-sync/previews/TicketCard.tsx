import { TicketCard } from '@takomo/web'

const noop = () => {}
const S = { fromSchedule: 'from schedule', notFulfilled: 'not fulfilled' }
const ago = (ms: number) => new Date(Date.now() - ms).toISOString()

function t(over: Record<string, unknown> = {}) {
  return {
    id: 'demo-2cx4',
    project: 'demo',
    title: 'Migrate off the billing_v1 table',
    state: 'needs-decision',
    priority: 'critical',
    tags: ['billing', 'migration'],
    updated_at: ago(3_600_000),
    ...over,
  }
}

const frame: React.CSSProperties = { width: 280, display: 'flex', flexDirection: 'column', gap: 8 }

/** The urgency scale: four bars AND the coloured word, so it reads either way. */
export function ByPriority() {
  return (
    <div style={frame}>
      <TicketCard ticket={t()} scheduleLabels={S} onOpen={noop} />
      <TicketCard ticket={t({ id: 'demo-prpw', priority: 'high', title: 'Apply the 2026 price list' })} scheduleLabels={S} onOpen={noop} />
      <TicketCard ticket={t({ id: 'demo-9qbw', priority: 'normal', title: 'Rate-limit POST /v1/questions' })} scheduleLabels={S} onOpen={noop} />
      <TicketCard ticket={t({ id: 'demo-7pec', priority: 'low', title: 'Spike: Slack socket-mode transport' })} scheduleLabels={S} onOpen={noop} />
    </div>
  )
}

/** Claimed, blocked, and selected. */
export function States() {
  return (
    <div style={frame}>
      <TicketCard
        ticket={t({ claim: { holder: 'agent:w1' } })}
        scheduleLabels={S}
        onOpen={noop}
      />
      <TicketCard
        ticket={t({ id: 'demo-2vwz', blocked_by: ['demo-3l2j'], title: 'Answer-link expiry sweep flakes' })}
        blockedLabel="Blocked — {n} open"
        scheduleLabels={S}
        onOpen={noop}
      />
      <TicketCard ticket={t()} selected scheduleLabels={S} onOpen={noop} />
    </div>
  )
}

/**
 * A scheduled ticket says where it came from — and an occurrence whose deadline
 * passed is flagged HERE, because expiry transitions nothing server-side and
 * this card is the only place a reader learns it stopped counting as live work.
 */
export function FromASchedule() {
  return (
    <div style={frame}>
      <TicketCard
        ticket={t({ id: 'demo-uxpa', title: 'Weekly review — 2026-W32', priority: 'normal', tags: ['ritual'], schedule: 'sch-4xz7evxn' })}
        scheduleLabels={S}
        onOpen={noop}
      />
      <TicketCard
        ticket={t({
          id: 'demo-bnzg',
          title: 'Weekly review — 2026-W27',
          priority: 'normal',
          tags: ['ritual'],
          schedule: 'sch-4xz7evxn',
          expires_at: ago(86_400_000),
        })}
        scheduleLabels={S}
        onOpen={noop}
      />
    </div>
  )
}
