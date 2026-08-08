import { UndoSnackbar } from '@takomo/web'

const noop = () => {}
const now = 1_000_000

/**
 * One snack per pending answer, stacked. Collapsing them into one would make
 * the second-to-last decision impossible to undo — and working an inbox quickly
 * leaves several windows running side by side.
 */
export function SeveralWindows() {
  return (
    <div style={{ position: 'relative', height: 260 }}>
      <UndoSnackbar
        now={now}
        pending={[
          {
            qid: 'q-1',
            payload: { value: true },
            decision: 'Decision: Confirm',
            detail: 'demo-2cx4 resumed',
            blocking: true,
            deadline: now + 27_000,
          },
          {
            qid: 'q-2',
            payload: { value: 'incremental' },
            decision: 'Decision: incremental',
            detail: 'Recorded — decision logged',
            blocking: false,
            deadline: now + 9_000,
          },
          {
            qid: 'q-5',
            payload: { value: true },
            decision: 'Decision: Approve',
            detail: 'demo-prpw resumed',
            blocking: true,
            deadline: now + 4_000,
          },
        ]}
        labels={{ undo: 'Undo', seconds: 's' }}
        onUndo={noop}
      />
    </div>
  )
}

/** A blocking answer resumes its ticket; an advisory one is only recorded. */
export function BlockingVsAdvisory() {
  return (
    <div style={{ position: 'relative', height: 180 }}>
      <UndoSnackbar
        now={now}
        pending={[
          {
            qid: 'q-3',
            payload: { value: false },
            decision: 'Decision: Reject',
            detail: 'demo-prpw resumed',
            blocking: true,
            deadline: now + 30_000,
          },
          {
            qid: 'q-4',
            payload: { value: 'ledger' },
            decision: 'Decision: ledger',
            detail: 'Recorded — decision logged',
            blocking: false,
            deadline: now + 24_000,
          },
        ]}
        labels={{ undo: 'Undo', seconds: 's' }}
        onUndo={noop}
      />
    </div>
  )
}
