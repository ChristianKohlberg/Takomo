import { QuestionRow } from '@takomo/web'

const noop = () => {}
const LABELS = { advisory: 'advisory', askedBy: 'asked by', waitingAgent: 'ticket stalled' }
const ago = (ms: number) => new Date(Date.now() - ms).toISOString()

function q(over: Record<string, unknown> = {}) {
  return {
    id: 'q-1',
    project: 'demo',
    ticket: 'demo-3l2j',
    kind: 'confirm' as const,
    mode: 'blocking' as const,
    status: 'open' as const,
    title: 'OK to drop table billing_v1?',
    options: [],
    option_notes: [],
    multi: false,
    recommended_multi: [],
    expertise: ['domain:billing'],
    urgency: 'critical',
    asked_by: 'agent:runner-1',
    created_at: ago(3_600_000),
    ...over,
  }
}

const frame: React.CSSProperties = { width: 340, border: '1px solid var(--border)', borderRadius: 10, overflow: 'hidden' }

/** Urgency drives the left rule — the one place colour ranks work. */
export function ByUrgency() {
  return (
    <div style={frame}>
      <QuestionRow question={q()} selected={false} labels={LABELS} onSelect={noop} />
      <QuestionRow
        question={q({ id: 'q-2', urgency: 'high', title: 'Approve re-pricing 1,800 subscriptions?' })}
        selected={false}
        labels={LABELS}
        onSelect={noop}
      />
      <QuestionRow
        question={q({ id: 'q-3', urgency: 'normal', mode: 'advisory', title: 'Rewrite or incremental?' })}
        selected={false}
        labels={LABELS}
        onSelect={noop}
      />
    </div>
  )
}

/** Selected, and one bounced back to the agent — not the reader's turn. */
export function SelectedAndWaiting() {
  return (
    <div style={frame}>
      <QuestionRow question={q()} selected labels={LABELS} onSelect={noop} />
      <QuestionRow
        question={q({ id: 'q-4', awaiting: 'agent', title: 'Which idempotency key should the retry reuse?' })}
        selected={false}
        labels={LABELS}
        onSelect={noop}
      />
    </div>
  )
}

/** Just answered in this tab: tinted while its undo window runs. */
export function Landed() {
  return (
    <div style={frame}>
      <QuestionRow question={q()} selected={false} landed labels={LABELS} onSelect={noop} />
    </div>
  )
}
