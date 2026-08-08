import { AnswerArea } from '@takomo/web'

const noop = () => {}
const L = {
  yes: 'Confirm', no: 'Decline', writeOwn: 'or your own instruction',
  ownPlaceholder: 'Write your own instruction …', textPlaceholder: 'Type your answer…',
  recommends: 'recommends',
}
function q(over: Record<string, unknown> = {}) {
  return {
    id: 'q-1', project: 'demo', ticket: 'demo-1',
    kind: 'confirm' as const, mode: 'blocking' as const, status: 'open' as const,
    title: 'Q', options: [] as string[], option_notes: [] as string[],
    multi: false, recommended_multi: [] as string[], expertise: [] as string[],
    created_at: '2026-08-08T00:00:00Z', ...over,
  }
}
const frame: React.CSSProperties = { maxWidth: 520 }

/** Yes/no, with the agent's recommendation MARKED (★) rather than hidden. */
export function Confirm() {
  return (
    <div style={frame}>
      <AnswerArea
        question={q({ recommended: true, recommended_note: 'Backfill verified against a restore; rollback is a rename.' })}
        draft={undefined} onDraft={noop} labels={L}
      />
    </div>
  )
}

/** Choose, with the per-option rationale the agent supplied. */
export function Choose() {
  return (
    <div style={frame}>
      <AnswerArea
        question={q({
          kind: 'choose',
          options: ['rewrite', 'incremental'],
          option_notes: ['Clean ledger, ~6 week freeze.', 'Ships continuously, v1 schema lingers.'],
          recommended: 'incremental',
        })}
        draft={undefined} onDraft={noop} labels={L}
      />
    </div>
  )
}

/** Multi-select, pre-ticked from `recommended_multi`. */
export function MultiChoose() {
  return (
    <div style={frame}>
      <AnswerArea
        question={q({ kind: 'choose', multi: true, options: ['ledger', 'invoices', 'webhooks'], recommended_multi: ['ledger', 'webhooks'] })}
        draft={undefined} onDraft={noop} labels={L}
      />
    </div>
  )
}

/** Free text for a clarify. */
export function Clarify() {
  return (
    <div style={frame}>
      <AnswerArea question={q({ kind: 'clarify' })} draft={undefined} onDraft={noop} labels={L} />
    </div>
  )
}

/** Read-only: a token without `human` sees the decision but cannot make it. */
export function ReadOnly() {
  return (
    <div style={frame}>
      <AnswerArea question={q({ recommended: true })} draft={undefined} onDraft={noop} labels={L} disabled />
    </div>
  )
}
