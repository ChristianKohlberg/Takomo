import { DetailPanel } from '@takomo/web'

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
  state: 'State',
  claimedBy: 'claimed by',
  labels: 'Labels',
  tagsHdr: 'Tags',
  description: 'Description',
  noDescription: 'No description.',
  dependencies: 'Dependencies',
  blockedByRel: 'blocked by ',
  blocksRel: 'blocks ',
  links: 'Links',
  blockedN: 'Blocked — {n} open question(s)',
  answeringResumes: 'Answering resumes this ticket.',
  decisionRouted: 'A routed decision — no state change.',
  answerInInbox: 'Answer in Inbox →',
  inConvN: '{n} in conversation',
  inConvSub: 'Waiting on the agent to report back.',
  readThread: 'Read the thread →',
  askHuman: 'Ask a human',
  close: 'Close',
  promotions: 'Promotions',
  comments: 'Comments',
  noComments: 'No comments.',
  refLabel: 'ref ',
  agoSep: ' ago · ',
}

/** The full drawer: a blocked ticket with everything it carries. */
export function Blocked() {
  return (
    <div style={frame}>
      <DetailPanel
        canAsk
        questions={{ count: 1, blocking: 1, advisory: 0, conv: 0 }}
        ticket={{
          id: 'demo-2cx4',
          project: 'demo',
          title: 'Migrate off the billing_v1 table',
          state: 'needs-decision',
          priority: 'critical',
          type: 'task',
          body: 'No reads in **90 days**. The copy-forward is ready and verified on a restore.\n\n- drop after the freeze\n- keep the rename as rollback',
          labels: ['migration'],
          tags: ['billing'],
          claim: { holder: 'agent:w1' },
          blocked_by: ['demo-3l2j'],
          links: {
            commit: 'https://github.com/ChristianKohlberg/Takomo/commit/9f2c1ab44d0e7b3c5a81',
            spec: 'https://example.com/spec',
          },
          promotions: [
            {
              target: 'production',
              actor: 'human:ada',
              created_at: ago(7_200_000),
              url: 'https://example.com/deploy/912',
            },
          ],
          comments: [
            {
              author: 'agent:w1',
              body: 'Backfill verified against a restore.',
              created_at: ago(3_600_000),
            },
            {
              author: 'human:ada',
              body: 'Hold until the freeze lifts.',
              created_at: ago(1_800_000),
            },
          ],
        }}
        labels={L}
        onClose={noop}
        onAsk={noop}
      />
    </div>
  )
}

/** Bounced back: "answering resumes this" would be a lie, so it does not say it. */
export function InConversation() {
  return (
    <div style={frame}>
      <DetailPanel
        canAsk={false}
        questions={{ count: 1, blocking: 0, advisory: 0, conv: 1 }}
        ticket={{
          id: 'demo-3l2j',
          project: 'demo',
          title: 'Webhook retries double-charge on 5xx',
          state: 'needs-decision',
          priority: 'critical',
          body: 'The provider accepts one key per capture attempt.',
          tags: ['billing', 'bug'],
        }}
        labels={L}
        onClose={noop}
        onAsk={noop}
      />
    </div>
  )
}
