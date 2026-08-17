import { describe, it, expect, afterEach } from 'vitest'
import { render, screen, cleanup, act } from '@testing-library/react'
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
  assignTo: 'Waiting on',
  assignNobody: 'Nobody yet',
  assignHint: 'Address this decision to one person.',
  msgMore: 'Show full reply',
  msgLess: 'Collapse reply',
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

describe('ReadingPane thread expansion', () => {
  const open: Question = { ...answered, status: 'open' }

  it('clamps a long reply behind a toggle that survives re-render', () => {
    const long = 'line\n'.repeat(20)
    const { rerender } = render(
      <ReadingPane
        question={open}
        thread={[{ id: 'm1', role: 'agent', body: long, author: 'agent', created_at: '2026-08-08T00:00:00Z' }]}
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
    const expand = screen.getByRole('button', { name: 'Show full reply' })
    act(() => expand.click())
    expect(screen.getByRole('button', { name: 'Collapse reply' })).toBeTruthy()
    rerender(
      <ReadingPane
        question={open}
        thread={[{ id: 'm1', role: 'agent', body: long, author: 'agent', created_at: '2026-08-08T00:00:00Z' }]}
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
    expect(screen.getByRole('button', { name: 'Collapse reply' })).toBeTruthy()
  })
})
