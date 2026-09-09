import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import * as Y from 'yjs'
import { createNode, readNodes } from '@/lib/mindmap-crdt'
import { Outline, type OutlineProps } from './Outline'

function setup() {
  const doc = new Y.Doc()
  const id = createNode(doc, { parent: null, title: 'A readable section title', by: 'test' })!
  const second = createNode(doc, { parent: id, title: 'A second thought', by: 'test' })!
  const props: OutlineProps = { nodes: readNodes(doc), selected: null, canWrite: true, naming: null,
    onSelect: vi.fn(), onEdit: vi.fn(), onRename: vi.fn(), onNameCommit: vi.fn(), onNameCancel: vi.fn(),
    onChild: vi.fn(), onSibling: vi.fn(), onAttachments: vi.fn(), onDelete: vi.fn(), onDetach: vi.fn(),
    foldSummaryOf: () => null, trustLens: false,
    labels: { actions: 'Section actions', edit: 'Open', rename: 'Rename', nameField: 'Title', nameHint: 'Title', addChild: 'Add child', addSibling: 'Add sibling', empty: 'Empty', hasNotes: 'Notes', attachments: 'Attachments {n}', remove: 'Remove', detach: 'Detach', folded: '{n} folded', question: 'Question', trustConfirmed: 'Confirmed', trustMachine: 'Machine', trustUnverified: 'Unverified' } }
  return { id, second, props }
}

it('keeps node actions behind a labeled menu and preserves root child creation', () => {
  const { id, props } = setup()
  render(<Outline {...props} />)
  expect(screen.getByRole('button', { name: 'A readable section title' })).toBeTruthy()
  expect(screen.queryByRole('button', { name: 'Add child' })).toBeNull()
  fireEvent.click(screen.getAllByLabelText('Section actions')[0]!)
  fireEvent.click(screen.getByRole('button', { name: 'Add child' }))
  expect(props.onChild).toHaveBeenCalledWith(id)
  expect(screen.queryByRole('button', { name: 'Add child' })).toBeNull()
})

it('opens one row menu at a time and dismisses it on Escape, returning focus to its trigger', async () => {
  const { second, props } = setup()
  render(<Outline {...props} />)
  const [first, other] = screen.getAllByLabelText('Section actions')
  fireEvent.click(first!)
  expect(screen.getAllByRole('button', { name: 'Add child' })).toHaveLength(1)
  fireEvent.click(other!)
  expect(screen.getAllByRole('button', { name: 'Add child' })).toHaveLength(1)
  fireEvent.click(screen.getByRole('button', { name: 'Add child' }))
  expect(props.onChild).toHaveBeenCalledWith(second)
  other!.focus()
  fireEvent.click(other!)
  expect(screen.getByRole('button', { name: 'Detach' })).toBeTruthy()
  fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Escape' })
  expect(screen.queryByRole('button', { name: 'Detach' })).toBeNull()
  await waitFor(() => expect(document.activeElement).toBe(other))
  expect(props.onDetach).not.toHaveBeenCalled()
})

it('highlights matches and scrolls the selected search result into view', () => {
  const { id, second, props } = setup()
  const scroll = vi.fn()
  const previous = HTMLElement.prototype.scrollIntoView
  HTMLElement.prototype.scrollIntoView = scroll
  try {
    const ui = render(<Outline {...props} searchMatches={new Set([id, second])} />)
    expect(ui.container.querySelectorAll('[data-search-match]')).toHaveLength(2)
    expect(scroll).not.toHaveBeenCalled()
    ui.rerender(<Outline {...props} selected={second} searchMatches={new Set([id, second])} />)
    expect(scroll).toHaveBeenCalledWith({ block: 'nearest' })
    ui.rerender(<Outline {...props} selected={second} searchMatches={new Set()} />)
    expect(ui.container.querySelector('[data-search-match]')).toBeNull()
  } finally { HTMLElement.prototype.scrollIntoView = previous }
})

it('offers focus from the phone menu even for readers', () => {
  const { id, props } = setup(), focus = vi.fn()
  render(<Outline {...props} canWrite={false} onFocusBranch={focus} />)
  fireEvent.click(screen.getAllByLabelText('Section actions')[0]!)
  fireEvent.click(screen.getByRole('button', { name: 'Focus branch' }))
  expect(focus).toHaveBeenCalledWith(id)
})
