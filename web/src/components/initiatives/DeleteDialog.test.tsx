// The confirmation before the one irreversible thing /initiatives can do.
//
// Two behaviours carry the weight: it says what is at stake instead of asking
// "are you sure", and it turns the server's check refusal into a SECOND explicit
// question rather than retrying with `force` behind the reader's back.
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
  checksTitle: 'Verification checks name it',
  checksBody: (n: number) => `${n} verification check(s) are filed under this initiative.`,
  checksForce: 'Deleting anyway keeps the checks and their cases.',
  confirm: 'Delete',
  confirmForce: 'Detach and delete',
  cancel: 'Cancel',
}

function ini(over: Partial<Initiative> = {}, rollup?: Rollup): Initiative {
  return { id: 'ini-a', project: 'demo', title: 'Billing', status: 'open', rollup, ...over }
}

function show(
  initiative: Initiative | null,
  onDelete: (id: string, force: boolean) => Promise<string[] | null>,
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

  it('deletes without force on the first confirmation', async () => {
    const onDelete = vi.fn().mockResolvedValue(null)
    show(ini(), onDelete)
    fireEvent.click(screen.getByRole('button', { name: labels.confirm }))
    await waitFor(() => expect(onDelete).toHaveBeenCalledWith('ini-a', false))
  })

  // The refusal is a question, not an error: keep the checks (detached) or stop.
  // Retrying with force automatically would make one click mean two things.
  it('turns a check refusal into a second, explicit confirmation', async () => {
    const onDelete = vi
      .fn()
      .mockResolvedValueOnce(['lane-a', 'lane-b'])
      .mockResolvedValueOnce(null)
    show(ini(), onDelete)

    fireEvent.click(screen.getByRole('button', { name: labels.confirm }))
    await waitFor(() => expect(screen.getByText(labels.checksTitle)).toBeTruthy())
    expect(onDelete).toHaveBeenCalledTimes(1)
    expect(onDelete).toHaveBeenLastCalledWith('ini-a', false)

    // It names them, and says what forcing actually does to them.
    expect(screen.getByText('lane-a')).toBeTruthy()
    expect(screen.getByText('lane-b')).toBeTruthy()
    expect(screen.getByText(labels.checksForce)).toBeTruthy()
    expect(screen.getByText('2 verification check(s) are filed under this initiative.')).toBeTruthy()

    fireEvent.click(screen.getByRole('button', { name: labels.confirmForce }))
    await waitFor(() => expect(onDelete).toHaveBeenCalledTimes(2))
    expect(onDelete).toHaveBeenLastCalledWith('ini-a', true)
  })

  it('cancels without deleting anything', () => {
    const onDelete = vi.fn()
    const { onOpenChange } = show(ini(), onDelete)
    fireEvent.click(screen.getByRole('button', { name: labels.cancel }))
    expect(onOpenChange).toHaveBeenCalledWith(false)
    expect(onDelete).not.toHaveBeenCalled()
  })

  // A refusal about one document must not greet the reader on the next one.
  it('forgets a refusal when it opens on a different document', async () => {
    const onDelete = vi.fn().mockResolvedValue(['lane-a'])
    const { onOpenChange } = show(ini(), onDelete)
    fireEvent.click(screen.getByRole('button', { name: labels.confirm }))
    await waitFor(() => expect(screen.getByText(labels.checksTitle)).toBeTruthy())
    cleanup()

    render(
      <DeleteDialog
        initiative={ini({ id: 'ini-b', title: 'Onboarding' })}
        onOpenChange={onOpenChange}
        onDelete={onDelete}
        labels={labels}
      />,
    )
    expect(screen.getByText(labels.title)).toBeTruthy()
    expect(screen.queryByText(labels.checksTitle)).toBeNull()
  })
})
