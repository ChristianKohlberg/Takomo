import { ReadingPane } from '@takomo/web'

const noop = () => {}
const ago = (ms: number) => new Date(Date.now() - ms).toISOString()
const L = {
  yes: 'Confirm', no: 'Decline', writeOwn: 'or your own instruction',
  ownPlaceholder: 'Write your own instruction …', textPlaceholder: 'Type your answer…',
  recommends: 'recommends', submit: 'Submit', sendFollow: 'Send follow-up',
  askFollow: 'Ask a follow-up…', followFirst: 'Type a follow-up first.',
  to: 'to', typeFirst: 'Type an answer first.', sendFirst: 'Select an answer first.',
  share: 'Create answer link', withdraw: 'Withdraw', reopen: 'Reopen',
  closed: 'This question is closed.', advisory: 'advisory', askedBy: 'asked by',
  readonly: "This token can't answer (no 'human' scope).",
  waitingAgentPrefix: 'Waiting on ', waitingAgentSuffix: ' — you asked for more context.',
  noReply: ' was notified. The reply appears here.',
}
function q(over: Record<string, unknown> = {}) {
  return {
    id: 'q-1', project: 'demo', ticket: 'demo-2cx4',
    kind: 'confirm' as const, mode: 'blocking' as const, status: 'open' as const,
    title: 'OK to drop table billing_v1?',
    body: 'No reads in **90 days**. I have the copy-forward ready and verified on a restore.',
    options: [] as string[], option_notes: [] as string[], multi: false,
    recommended_multi: [] as string[], expertise: ['domain:billing'],
    asked_by: 'agent:runner-1', urgency: 'critical',
    created_at: ago(3_600_000), ...over,
  }
}
const frame: React.CSSProperties = { height: 560, display: 'flex', border: '1px solid var(--border)', borderRadius: 10, overflow: 'hidden' }

/** A question waiting on a decision, with the recommendation pre-arming Submit. */
export function Open() {
  return (
    <div style={frame}>
      <ReadingPane
        question={q({ recommended: true, recommended_note: 'Backfill verified; rollback is a rename.' })}
        thread={[]} draft={undefined} onDraft={noop} canAnswer labels={L}
        onSubmit={noop} onFollowup={noop} onWithdraw={noop} onReopen={noop} onShare={noop}
      />
    </div>
  )
}

/** Bounced back to the agent: the thread, and who it is waiting on. */
export function InConversation() {
  return (
    <div style={frame}>
      <ReadingPane
        question={q({
          kind: 'clarify',
          awaiting: 'agent',
          title: 'Which idempotency key should the retry reuse?',
          body: 'The provider accepts one key per capture attempt. Reusing it across a 5xx retry is what decides whether a customer gets charged twice.',
        })}
        thread={[{ id: 'm1', role: 'human', body: 'Does the provider treat a re-used key as idempotent across attempts?', author: 'human:ada', created_at: ago(1_800_000) }]}
        draft={undefined} onDraft={noop} canAnswer labels={L}
        onSubmit={noop} onFollowup={noop} onWithdraw={noop} onReopen={noop} onShare={noop}
      />
    </div>
  )
}

/** Closed: only an ANSWERED question can be reopened, so nothing else offers it. */
export function Answered() {
  return (
    <div style={frame}>
      <ReadingPane
        question={q({ status: 'answered', answered_by: 'human:ada' })}
        thread={[]} draft={undefined} onDraft={noop} canAnswer labels={L}
        onSubmit={noop} onFollowup={noop} onWithdraw={noop} onReopen={noop} onShare={noop}
      />
    </div>
  )
}
