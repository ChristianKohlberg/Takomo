import { describe, it, expect, afterEach } from 'vitest'
import { render, screen, cleanup } from '@testing-library/react'
import { ReadingPane } from './ReadingPane'
import type { Question } from '@/lib/questions'

afterEach(cleanup)

const answered: Question = {
  id: 'q-1',
  project: 'demo',
  ticket: 'demo-1',
  kind: 'confirm',
  mode: 'blocking',
  status: 'answered',
  title: 'Ship it?',
  options: [],
  option_notes: [],
  multi: false,
  recommended_multi: [],
  expertise: [],
  awaiting: null,
  created_at: '2026-08-08T00:00:00Z',
  answer: { value: true },
}

const labels = {
  submit: 'Submit',
  sendFollow: 'Send',
  askFollow: 'Ask',
  followFirst: 'Type first',
  to: 'To',
  typeFirst: 'Type',
  sendFirst: 'Send first',
  share: 'Share',
  withdraw: 'Withdraw',
  reopen: 'Reopen',
  closed: 'Closed',
  advisory: 'Advisory',
  askedBy: 'Asked by',
  readonly: 'Read-only',
  waitingAgentPrefix: 'Waiting on ',
  waitingAgentSuffix: ' to reply',
  noReply: 'No reply',
  yes: 'Yes',
  no: 'No',
  writeOwn: 'Other',
  ownPlaceholder: '…',
  textPlaceholder: '…',
  recommends: 'Agent suggests',
}

describe('ReadingPane reopen', () => {
  it('hides Reopen while an optimistic answer is still pending', () => {
    render(
      <ReadingPane
        question={answered}
        thread={[]}
        draft={undefined}
        onDraft={() => {}}
        canAnswer
        labels={labels}
        onSubmit={() => {}}
        onFollowup={() => {}}
        onWithdraw={() => {}}
        onReopen={() => {}}
        onShare={() => {}}
        answerPending
      />,
    )
    expect(screen.queryByRole('button', { name: 'Reopen' })).toBeNull()
  })

  it('offers Reopen for a landed answer with no pending window', () => {
    render(
      <ReadingPane
        question={answered}
        thread={[]}
        draft={undefined}
        onDraft={() => {}}
        canAnswer
        labels={labels}
        onSubmit={() => {}}
        onFollowup={() => {}}
        onWithdraw={() => {}}
        onReopen={() => {}}
        onShare={() => {}}
      />,
    )
    expect(screen.getByRole('button', { name: 'Reopen' })).toBeTruthy()
  })
})
