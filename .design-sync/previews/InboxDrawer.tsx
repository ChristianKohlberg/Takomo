import { InboxDrawer } from '@takomo/web'

const noop = () => {}

// Both stories render a `position: fixed` drawer. A `transform` on an ancestor
// makes that ancestor a containing block for fixed descendants, which is what
// gives the drawer a real box inside a preview card without changing the
// component (the render check catches the alternative as a 0px height).
const frame: React.CSSProperties = {
  position: 'relative',
  height: 720,
  width: '100%',
  transform: 'translateZ(0)',
  overflow: 'hidden',
  background: 'var(--background)',
}

const ago = (ms: number) => new Date(Date.now() - ms).toISOString()
const L = {
  title: 'full inbox →',
  empty: 'All clear',
  emptySub: 'No open questions. The fleet is working.',
  blocking: 'blocking',
  advisory: 'advisory',
  inConversation: 'in conversation',
  awaiting: 'Waiting on ',
  awaitingSub: ' — you asked for more context.',
  recommends: 'recommends',
  notePlaceholder: 'Add a note (optional)…',
  send: 'Send',
  cantAnswer: "This token can't answer (no 'human' scope).",
  close: 'Close',
  approve: 'Approve',
  reject: 'Reject',
  yes: 'Yes',
  no: 'No',
  writeOwn: 'or your own instruction',
  ownPlaceholder: 'Write your own …',
  textPlaceholder: 'Type your answer…',
  typeFirst: 'Type an answer first.',
  sendFirst: 'Select an answer first.',
}
function q(over: Record<string, unknown> = {}) {
  return {
    id: 'q-1',
    project: 'demo',
    ticket: 'demo-2cx4',
    kind: 'confirm' as const,
    mode: 'blocking' as const,
    status: 'open' as const,
    title: 'OK to drop table billing_v1?',
    body: 'No reads in 90 days.',
    options: [] as string[],
    option_notes: [] as string[],
    multi: false,
    recommended_multi: [] as string[],
    expertise: [] as string[],
    asked_by: 'agent:runner-1',
    urgency: 'critical',
    created_at: ago(3_600_000),
    ...over,
  }
}

/** Decisions reachable from where you noticed them, without leaving the board. */
export function WithQuestions() {
  return (
    <div style={frame}>
      <InboxDrawer
        open
        canAnswer
        questions={[
          q({ recommended: true }),
          q({
            id: 'q-2',
            kind: 'approve',
            urgency: 'high',
            title: 'Approve re-pricing 1,800 live subscriptions?',
            ticket: 'demo-prpw',
          }),
          q({
            id: 'q-3',
            mode: 'advisory',
            urgency: 'normal',
            kind: 'choose',
            options: ['rewrite', 'incremental'],
            recommended: 'incremental',
            title: 'Rewrite or incremental?',
            ticket: 'demo-0lj3',
          }),
        ]}
        labels={L}
        onClose={noop}
        onAnswer={async () => {}}
      />
    </div>
  )
}

/** Nothing to decide — said plainly rather than shown as an empty list. */
export function AllClear() {
  return (
    <div style={frame}>
      <InboxDrawer
        open
        canAnswer
        questions={[]}
        labels={L}
        onClose={noop}
        onAnswer={async () => {}}
      />
    </div>
  )
}
