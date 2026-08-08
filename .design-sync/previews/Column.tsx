import { Column } from '@takomo/web'

const noop = () => {}
const LABELS = { showMore: '+{n} more', blocked: 'Blocked — {n} open', fromSchedule: 'from schedule', notFulfilled: 'not fulfilled' }
const ago = (ms: number) => new Date(Date.now() - ms).toISOString()

const mk = (n: number, over: Record<string, unknown> = {}) =>
  Array.from({ length: n }, (_, i) => ({
    id: `demo-${1000 + i}`,
    project: 'demo',
    title: ['Weekly review — 2026-W33', 'Rate-limit POST /v1/questions per token', 'Port the Aquarelle palette', 'Drop the legacy redirect', 'Serve the octopus favicon', 'Spike: Slack socket-mode', 'Migrate off billing_v1'][i % 7]!,
    state: 'brief',
    priority: (['critical', 'high', 'normal', 'low'] as const)[i % 4]!,
    tags: i % 2 ? ['ritual'] : ['api'],
    updated_at: ago((i + 1) * 3_600_000),
    ...over,
  }))

/** Columns come from the PROJECT'S workflow, not a fixed todo/doing/done. */
export function Populated() {
  return (
    <div style={{ display: 'flex', gap: 12 }}>
      <Column state="brief" tickets={mk(3)} labels={LABELS} onOpen={noop} />
      <Column state="implementing" tickets={mk(2, { state: 'implementing', claim: { holder: 'agent:w1' } })} labels={LABELS} onOpen={noop} />
    </div>
  )
}

/** Past six cards the rest collapse, so one busy column cannot bury the others. */
export function Collapsed() {
  return <Column state="brief" tickets={mk(11)} labels={LABELS} onOpen={noop} />
}

/** An empty state is a real state — the column keeps its place in the board. */
export function Empty() {
  return <Column state="review" tickets={[]} labels={LABELS} onOpen={noop} />
}
