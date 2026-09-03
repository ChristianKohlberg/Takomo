// The panel where "an agent proposes, a person confirms" becomes two buttons.
//
// What is worth a test here is not the layout but the three rules the panel is
// there to keep: the reason is readable before the diff, what the server dropped
// is visible, and a reader who cannot change the plan is offered no decision at
// all.
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { ProposalPanel } from './ProposalPanel'
import type { Proposal } from '@/lib/doc-ops'

const labels = {
  heading: 'What an agent offers here',
  empty: 'Nothing has been offered for this section.',
  pending: 'undecided',
  accepted: 'accepted',
  rejected: 'rejected',
  accept: 'Accept',
  reject: 'Reject',
  by: 'from',
  partial: 'The server dropped part of this before you saw it:',
  opReplace: 'rewrites',
  opInsert: 'adds after',
  opDelete: 'removes',
  readOnly: 'Deciding needs a token that may change the plan.',
}

function proposal(over: Partial<Proposal> = {}): Proposal {
  return {
    id: 'prop-a',
    node: 'mn-1',
    status: 'pending',
    author: 'agent:fleet-3',
    instruction: 'Tighten the billing section.',
    summary: 'It promises monthly billing that section 4 takes back.',
    created_at: 1,
    skipped: [],
    ops: [{ op: 'replace', id: 'blk_b', markdown: 'Billing runs monthly.' }],
    ...over,
  }
}

function panel(over: Partial<Parameters<typeof ProposalPanel>[0]> = {}) {
  const props = {
    proposals: [proposal()],
    textFor: (id: string) => (id === 'blk_b' ? 'Billing, one day.' : null),
    canWrite: true,
    onAccept: vi.fn(),
    onReject: vi.fn(),
    labels,
    ...over,
  }
  return { ...render(<ProposalPanel {...props} />), props }
}

describe('ProposalPanel', () => {
  it('says nothing has been offered rather than showing an empty list', () => {
    panel({ proposals: [] })
    expect(screen.getByText(labels.empty)).toBeTruthy()
  })

  it('leads with the reason, and shows the change under it', () => {
    panel()
    expect(screen.getByText(/section 4 takes back/)).toBeTruthy()
    // The before-side comes from the document as it stands now, not from the
    // proposal — which is what makes it a diff rather than a quotation.
    expect(screen.getByText('Billing, one day.')).toBeTruthy()
    expect(screen.getByText('Billing runs monthly.')).toBeTruthy()
    expect(screen.getByText(/rewrites · blk_b/)).toBeTruthy()
  })

  it('shows what the server dropped, so a partial change is not accepted as a whole one', () => {
    panel({
      proposals: [proposal({ skipped: ['replace blk_z: outside the scope of this proposal'] })],
    })
    expect(screen.getByText(/The server dropped part of this/)).toBeTruthy()
    expect(screen.getByText(/blk_z: outside the scope/)).toBeTruthy()
  })

  it('offers accept and reject to somebody who may change the plan', () => {
    const { props } = panel()
    fireEvent.click(screen.getByRole('button', { name: labels.accept }))
    expect(props.onAccept).toHaveBeenCalledWith(expect.objectContaining({ id: 'prop-a' }))
    fireEvent.click(screen.getByRole('button', { name: labels.reject }))
    expect(props.onReject).toHaveBeenCalledWith(expect.objectContaining({ id: 'prop-a' }))
  })

  it('offers a reader no decision at all, and still shows what is proposed', () => {
    panel({ canWrite: false })
    expect(screen.queryByRole('button', { name: labels.accept })).toBeNull()
    expect(screen.queryByRole('button', { name: labels.reject })).toBeNull()
    expect(screen.getByText(labels.readOnly)).toBeTruthy()
    expect(screen.getByText(/section 4 takes back/)).toBeTruthy()
  })

  it('keeps a decided proposal readable, with who decided it', () => {
    panel({
      proposals: [proposal({ status: 'rejected', decided_by: 'Ada' })],
    })
    expect(screen.getByText(labels.rejected)).toBeTruthy()
    expect(screen.getByText(/Ada/)).toBeTruthy()
    // Nothing to decide twice.
    expect(screen.queryByRole('button', { name: labels.accept })).toBeNull()
  })
})
