import { fireEvent, render, screen } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import * as Y from 'yjs'
import { createNode, readNodes } from '@/lib/mindmap-crdt'
import { Outline, type OutlineProps } from './Outline'

it('keeps node actions behind a labeled menu and preserves root child creation', () => {
  const doc = new Y.Doc()
  const id = createNode(doc, { parent: null, title: 'A readable section title', by: 'test' })!
  const props: OutlineProps = { nodes: readNodes(doc), selected: null, canWrite: true, naming: null,
    onSelect: vi.fn(), onEdit: vi.fn(), onRename: vi.fn(), onNameCommit: vi.fn(), onNameCancel: vi.fn(),
    onChild: vi.fn(), onSibling: vi.fn(), onAttachments: vi.fn(), onDelete: vi.fn(), onDetach: vi.fn(),
    foldSummaryOf: () => null, trustLens: false,
    labels: { actions: 'Section actions', edit: 'Open', rename: 'Rename', nameField: 'Title', nameHint: 'Title', addChild: 'Add child', addSibling: 'Add sibling', empty: 'Empty', hasNotes: 'Notes', attachments: 'Attachments {n}', remove: 'Remove', detach: 'Detach', folded: '{n} folded', question: 'Question', trustConfirmed: 'Confirmed', trustMachine: 'Machine', trustUnverified: 'Unverified' } }
  render(<Outline {...props} />)
  expect(screen.getByRole('button', { name: 'A readable section title' })).toBeTruthy()
  expect(screen.getByRole('button', { name: 'Add child' }).closest('details')?.open).toBe(false)
  fireEvent.click(screen.getByLabelText('Section actions'))
  fireEvent.click(screen.getByRole('button', { name: 'Add child' }))
  expect(props.onChild).toHaveBeenCalledWith(id)
  expect(screen.getByRole('button', { name: 'Add child' }).closest('details')?.open).toBe(false)
})
