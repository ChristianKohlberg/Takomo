// The manager's rules, not its looks — jsdom has no layout engine, so nothing
// here says the dialog is drawn correctly. What it does say is that a read-only
// token cannot reach a write, that a full node is told so with the message the
// rest of the surface uses, and that correcting an attachment updates the one it
// was opened on rather than adding a second.
import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'

import { AttachmentsDialog, type AttachmentsDialogLabels } from './AttachmentsDialog'
import { MAX_ATTACHMENTS, type Attachment } from '@/lib/mindmap-doc'
import { STR } from '@/pages/mindmap/strings'

const LABELS: AttachmentsDialogLabels = {
  title: 'Attached to “{title}”',
  subtitle: 'Pointers to things that live elsewhere.',
  empty: 'Nothing attached yet.',
  count: '{n} of {max}',
  full: 'This thought is at its {max} attachments.',
  kind: 'Kind',
  name: 'Name',
  gist: 'Gist',
  ref: 'URL or path',
  add: 'Add attachment',
  addOpen: 'Attach something',
  edit: 'Edit',
  save: 'Save',
  remove: 'Remove attachment',
  cancel: 'Cancel',
  close: 'Close',
  readOnly: 'This token cannot change anything here.',
  kinds: {
    pdf: 'PDF',
    code: 'Code',
    table: 'Table',
    diagram: 'Diagram',
    audio: 'Audio',
    link: 'Link',
  },
}

const attachment = (over: Partial<Attachment> = {}): Attachment => ({
  id: 'ma-1',
  kind: 'pdf',
  name: 'spec.pdf',
  gist: 'the pricing rules',
  ref: 'https://example.com/spec.pdf',
  ...over,
})

const nodeWith = (attachments: Attachment[]) => ({
  id: 'mn-1',
  title: 'Pricing',
  attachments,
})

function mount(over: Partial<Parameters<typeof AttachmentsDialog>[0]> = {}) {
  const props = {
    node: nodeWith([attachment()]),
    canWrite: true,
    onOpenChange: vi.fn(),
    onAdd: vi.fn(),
    onUpdate: vi.fn(),
    onRemove: vi.fn(),
    labels: LABELS,
    ...over,
  }
  render(<AttachmentsDialog {...props} />)
  return props
}

describe('AttachmentsDialog', () => {
  it('lists what is there, with its kind, ref and gist', () => {
    mount()
    expect(screen.getByText('spec.pdf')).toBeTruthy()
    expect(screen.getByText('PDF')).toBeTruthy()
    expect(screen.getByText('https://example.com/spec.pdf')).toBeTruthy()
    expect(screen.getByText('the pricing rules')).toBeTruthy()
  })

  it('says so when there is nothing yet, rather than showing an empty box', () => {
    mount({ node: nodeWith([]) })
    expect(screen.getByText('Nothing attached yet.')).toBeTruthy()
  })

  it('adds what the form was filled with', () => {
    const props = mount({ node: nodeWith([]) })
    fireEvent.click(screen.getByText('+ Attach something'))
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: '  notes.md  ' } })
    fireEvent.change(screen.getByLabelText('URL or path'), { target: { value: ' /docs ' } })
    fireEvent.click(screen.getByText('Add attachment'))
    expect(props.onAdd).toHaveBeenCalledWith('mn-1', {
      kind: 'link',
      name: 'notes.md',
      gist: '',
      ref: '/docs',
    })
    expect(props.onUpdate).not.toHaveBeenCalled()
  })

  it('corrects the attachment it was opened on rather than adding a second', () => {
    // The id is what every other peer is holding. Remove-then-add would read, on
    // the other side of the socket, as a deletion and an unrelated addition.
    const props = mount()
    fireEvent.click(screen.getByLabelText('Edit — spec.pdf'))
    fireEvent.change(screen.getByLabelText('Name'), { target: { value: 'spec v2.pdf' } })
    fireEvent.click(screen.getByText('Save'))
    expect(props.onUpdate).toHaveBeenCalledWith('mn-1', 'ma-1', {
      kind: 'pdf',
      name: 'spec v2.pdf',
      gist: 'the pricing rules',
      ref: 'https://example.com/spec.pdf',
    })
    expect(props.onAdd).not.toHaveBeenCalled()
  })

  it('removes the one that was asked for', () => {
    const props = mount()
    fireEvent.click(screen.getByLabelText('Remove attachment — spec.pdf'))
    expect(props.onRemove).toHaveBeenCalledWith('mn-1', 'ma-1')
  })

  it('refuses a full node with the message the rest of the surface uses', () => {
    const full = Array.from({ length: MAX_ATTACHMENTS }, (_, i) =>
      attachment({ id: `ma-${i}`, name: `f${i}.pdf` }),
    )
    mount({ node: nodeWith(full) })
    expect(screen.getByText(`This thought is at its ${MAX_ATTACHMENTS} attachments.`)).toBeTruthy()
    expect(screen.getByText('+ Attach something').closest('button')?.disabled).toBe(true)
  })

  it('gives a read-only token the list and nothing to press', () => {
    mount({ canWrite: false })
    expect(screen.getByText('spec.pdf')).toBeTruthy()
    expect(screen.queryByLabelText('Edit — spec.pdf')).toBeNull()
    expect(screen.queryByText('+ Attach something')).toBeNull()
    expect(screen.getByText('This token cannot change anything here.')).toBeTruthy()
  })
})

describe('the cap message', () => {
  it('is the one string, carrying the placeholder both readers substitute', () => {
    // The drop path reports a refusal with the SAME sentence the dialog shows,
    // which is only true while it stays a template rather than a hardcoded 20.
    for (const table of [STR.en, STR.de]) {
      expect(table.attachmentsFull).toContain('{max}')
    }
  })
})
