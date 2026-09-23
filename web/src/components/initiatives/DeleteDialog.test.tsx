// The confirmation before the one irreversible thing /initiatives can do.
//
// It says what is at stake instead of asking "are you sure".
import { describe, it, expect, afterEach, vi } from 'vitest'
import { render, screen, cleanup, fireEvent, waitFor } from '@testing-library/react'
import { DeleteDialog } from './DeleteDialog'
import type { Initiative, Rollup } from '@/lib/initiatives'

afterEach(cleanup)

const labels = {
  title: 'Delete initiative',
  body: '“{title}” and everything appended to it will be removed.',
  contents: (n: number, a: number) => `It holds ${n} entry/entries, ${a} of them documents.`,
  stillWaiting: 'Someone is still waiting on an answer or a decision inside it.',
  taggedWork: (n: number) => `${n} ticket(s) are filed under it.`,
  irreversible: 'This cannot be undone.',
  confirm: 'Delete',
  cancel: 'Cancel',
}

function ini(over: Partial<Initiative> = {}, rollup?: Rollup): Initiative {
  return { id: 'ini-a', project: 'demo', title: 'Billing', status: 'open', rollup, ...over }
}

function show(
  initiative: Initiative | null,
  onDelete: (id: string) => Promise<void>,
  taggedTickets = 0,
) {
  const onOpenChange = vi.fn()
  render(
    <DeleteDialog
      initiative={initiative}
      onOpenChange={onOpenChange}
      onDelete={onDelete}
      taggedTickets={taggedTickets}
      labels={labels}
    />,
  )
  return { onOpenChange }
}

describe('DeleteDialog', () => {
  it('renders nothing when no document is being deleted', () => {
    show(null, vi.fn())
    expect(screen.queryByRole('dialog')).toBeNull()
  })

  // A tree row cannot distinguish an initiative fed for three months from one
  // opened by mistake yesterday. This is where that difference gets shown.
  it('says what is at stake rather than asking whether you are sure', () => {
    show(ini({}, { entries: 12, attachments: 3 }), vi.fn(), 4)
    expect(screen.getByText(/“Billing”/)).toBeTruthy()
    expect(screen.getByText('It holds 12 entry/entries, 3 of them documents.')).toBeTruthy()
    expect(screen.getByText('4 ticket(s) are filed under it.')).toBeTruthy()
    expect(screen.getByText(labels.irreversible)).toBeTruthy()
  })

  it('warns when someone is mid-conversation inside it', () => {
    show(ini({}, { entries: 3, open_notes: 1, pending_amendments: 0 }), vi.fn())
    expect(screen.getByText(labels.stillWaiting)).toBeTruthy()
  })

  it('stays quiet about waiting, contents and work when there is none', () => {
    show(ini({}, { entries: 0, attachments: 0 }), vi.fn())
    expect(screen.queryByText(labels.stillWaiting)).toBeNull()
    expect(screen.queryByText(/It holds/)).toBeNull()
    expect(screen.queryByText(/filed under it/)).toBeNull()
  })

  it('deletes on confirmation', async () => {
    const onDelete = vi.fn().mockResolvedValue(undefined)
    show(ini(), onDelete)
    fireEvent.click(screen.getByRole('button', { name: labels.confirm }))
    await waitFor(() => expect(onDelete).toHaveBeenCalledWith('ini-a'))
  })

  it('cancels without deleting anything', () => {
    const onDelete = vi.fn()
    const { onOpenChange } = show(ini(), onDelete)
    fireEvent.click(screen.getByRole('button', { name: labels.cancel }))
    expect(onOpenChange).toHaveBeenCalledWith(false)
    expect(onDelete).not.toHaveBeenCalled()
  })
})
