import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Awareness } from 'y-protocols/awareness'
import * as Y from 'yjs'
import Plan, { type PlanProps } from './Plan'
import { createStructureHistory } from '@/lib/plan-structure'
import { createNode, nodesMap, readPlanTree } from '@/lib/mindmap-crdt'
import type { Editor } from '@tiptap/react'

const probe = vi.hoisted(() => ({ editors: new Map<string, Editor>(), panelRenders: 0 }))
vi.mock('./SectionEditor', async (importOriginal) => {
  const mod = await importOriginal<typeof import('./SectionEditor')>()
  const Original = mod.default
  const Probed: typeof Original = (props) => <Original {...props} onEditor={(editor) => {
    if (editor) probe.editors.set(props.label, editor)
    else probe.editors.delete(props.label)
    props.onEditor?.(editor)
  }} />
  return { ...mod, default: Probed }
})
vi.mock('@/components/documents/SectionPanel', async (importOriginal) => {
  const mod = await importOriginal<typeof import('@/components/documents/SectionPanel')>()
  const Original = mod.SectionPanel
  const Probed: typeof Original = (props) => { probe.panelRenders++; return <Original {...props} /> }
  return { ...mod, SectionPanel: Probed }
})

const fixtures: { doc: Y.Doc; awareness: Awareness }[] = []
beforeEach(() => {
  localStorage.clear()
  probe.editors.clear()
  probe.panelRenders = 0
  Element.prototype.scrollIntoView = vi.fn()
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
})
afterEach(() => { vi.unstubAllGlobals(); for (const { doc, awareness } of fixtures.splice(0)) { awareness.destroy(); doc.destroy() } })

function setup() {
  const doc = new Y.Doc()
  const awareness = new Awareness(doc)
  fixtures.push({ doc, awareness })
  const a = createNode(doc, { title: 'Billing', parent: null, by: 'Ada' })!
  const child = createNode(doc, { title: 'Invoices', parent: a, by: 'Ada' })!
  const b = createNode(doc, { title: 'Reports', parent: null, by: 'Ada' })!
  const fragment = nodesMap(doc).get(child)!.get('prose') as Y.XmlFragment
  const paragraph = new Y.XmlElement('paragraph')
  paragraph.insert(0, [new Y.XmlText('Payment deadline is thirty days.')])
  fragment.insert(0, [paragraph])
  nodesMap(doc).get(child)!.set('test_link', 'test-invoices')
  const props: PlanProps = {
    connection: { ydoc: doc, provider: { awareness } } as PlanProps['connection'],
    session: { object: 'mm-workflow', kind: 'mindmap', mindmap: 'mm-workflow', session: 'test', token: '', can_write: true, display: 'Ada', expires_at: '', url: '', room: 'mm-workflow' },
    testsFor: () => ({ total: 1, failing: 0 }), onShowTests: vi.fn(), testsLabel: 'Tests', failedLabel: 'failed', onError: vi.fn(),
    standing: {}, trace: new Map(), onReview: vi.fn(), onEdited: vi.fn(), onShowOnMap: vi.fn(), onDecided: vi.fn(), onSkipped: vi.fn(), onSelection: vi.fn(),
    labels: { readOnly: 'Read only', empty: 'Empty', emptyHint: '', proseEmpty: 'Write here', proseLabel: 'Section {n} prose' },
    railLabels: { outline: 'Outline', expand: 'Expand section', collapse: 'Collapse section', folded: '{n} sections inside', untitled: 'Untitled', standingConfirmed: 'Agreed', standingChanged: 'Changed', standingUnseen: 'Unread', pending: '{n} pending' },
    sectionLabels: { renameSection: 'Rename section', untitled: 'Untitled', standingConfirmed: 'Agreed', standingChanged: 'Changed', standingUnseen: 'Unread', review: 'Reviewed', reviewHint: 'Mark reviewed', showOnMap: 'Show on map', history: 'History', hideHistory: 'Hide history', historyEmpty: 'No history', historyMore: '{n} older', proposals: 'Proposals', hideProposals: 'Hide proposals', pendingBadge: '{n} waiting', needWrite: 'Read only', kinds: { authored: 'Written', renamed: 'Renamed', edited: 'Edited', moved: 'Moved', pruned: 'Removed', reviewed: 'Reviewed', proposed: 'Proposed', accepted: 'Accepted', rejected: 'Rejected' } },
    proposalLabels: { heading: 'Proposals', empty: 'No proposals', pending: 'Pending', accepted: 'Accepted', rejected: 'Rejected', accept: 'Accept', reject: 'Reject', by: 'By', partial: 'Partial', opReplace: 'Replace', opInsert: 'Insert', opDelete: 'Delete', readOnly: 'Read only' },
  }
  return { doc, a, b, child, fragment, props }
}

describe('document workflow integration', () => {
  it('keeps find available to readers while withholding all structure mutations', () => {
    const { doc, props } = setup()
    render(<Plan {...props} session={{ ...props.session, can_write: false }} />)
    expect(screen.queryByRole('button', { name: 'Move section 1' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Undo section move' })).toBeNull()
    const update = vi.fn()
    doc.on('update', update)
    fireEvent.click(screen.getByRole('button', { name: 'Find in document' }))
    fireEvent.change(screen.getByRole('searchbox', { name: 'Find in document' }), { target: { value: 'deadline' } })
    expect(screen.getByText('1 / 1')).toBeTruthy()
    fireEvent.change(screen.getByRole('searchbox', { name: 'Find in document' }), { target: { value: 'Billing' } })
    expect(screen.getByText('1 / 1')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Close search' }))
    expect(update).not.toHaveBeenCalled()
  })

  it('moves a section through the outline, then undoes and redoes without replacing its subtree', async () => {
    const { doc, a, b, child, fragment, props } = setup()
    render(<Plan {...props} />)
    fireEvent.click(screen.getByRole('button', { name: 'Move section 1' }))
    fireEvent.change(screen.getByLabelText('Destination section'), { target: { value: b } })
    fireEvent.change(screen.getByLabelText('Position'), { target: { value: 'child' } })
    fireEvent.click(screen.getByRole('button', { name: 'Move' }))
    expect(readPlanTree(doc).find(n => n.id === a)?.parent).toBe(b)
    expect(props.onSelection).toHaveBeenLastCalledWith(a)
    expect(screen.queryByRole('dialog')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Undo section move' }))
    expect(readPlanTree(doc).find(n => n.id === a)?.parent).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Redo section move' }))
    expect(readPlanTree(doc).find(n => n.id === a)?.parent).toBe(b)
    expect(nodesMap(doc).get(child)!.get('parent')).toBe(a)
    expect(nodesMap(doc).get(child)!.get('prose')).toBe(fragment)
    expect(fragment.toString()).toContain('Payment deadline')
    expect(nodesMap(doc).get(child)!.get('test_link')).toBe('test-invoices')
    await waitFor(() => expect(screen.getByLabelText('Section 1.1.1 prose')).toBeTruthy())
  })

  it('reveals matching prose inside a collapsed section temporarily and preserves the stored fold', async () => {
    const { a, props } = setup()
    localStorage.setItem('takomo.plan.fold.mm-workflow', JSON.stringify([a]))
    render(<Plan {...props} />)
    expect(screen.queryByLabelText('Section 1.1 prose')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Find in document' }))
    fireEvent.change(screen.getByRole('searchbox', { name: 'Find in document' }), { target: { value: 'deadline' } })
    await waitFor(() => expect(screen.getByLabelText('Section 1.1 prose').textContent).toContain('Payment deadline'))
    expect(localStorage.getItem('takomo.plan.fold.mm-workflow')).toBe(JSON.stringify([a]))
    fireEvent.click(screen.getByRole('button', { name: 'Close search' }))
    await waitFor(() => expect(screen.queryByLabelText('Section 1.1 prose')).toBeNull())
    expect(localStorage.getItem('takomo.plan.fold.mm-workflow')).toBe(JSON.stringify([a]))
  })

  it('searches unmounted prose and mounts only the active match without repeating selection on edits', async () => {
    vi.stubGlobal('IntersectionObserver', class { observe() {} unobserve() {} disconnect() {} })
    const { fragment, props } = setup()
    render(<Plan {...props} />)
    expect(screen.queryByLabelText('Section 1.1 prose')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Find in document' }))
    fireEvent.change(screen.getByRole('searchbox', { name: 'Find in document' }), { target: { value: 'deadline' } })
    await waitFor(() => expect(screen.getByLabelText('Section 1.1 prose')).toBeTruthy())
    const count = vi.mocked(props.onSelection!).mock.calls.length
    act(() => { (fragment.get(0) as Y.XmlElement).insert(1, [new Y.XmlText(' More prose.')]) })
    expect(screen.getByLabelText('Section 1.1 prose').textContent).toContain('More prose.')
    expect(vi.mocked(props.onSelection!).mock.calls.length).toBe(count)
  })

  it('enables text undo for the selected section only, without re-rendering the plan on every keystroke', () => {
    const { props } = setup()
    render(<Plan {...props} />)
    const undo = screen.getByRole('button', { name: 'Undo section text' }) as HTMLButtonElement
    const invoices = probe.editors.get('Section 1.1 prose')!
    const billing = probe.editors.get('Section 1 prose')!
    expect(undo.disabled).toBe(true)
    act(() => { billing.commands.insertContentAt(1, 'Unselected. ') })
    expect(undo.disabled).toBe(true)
    fireEvent.pointerDown(screen.getByLabelText('Section 1.1 prose'))
    expect(undo.disabled).toBe(true)
    act(() => { invoices.commands.insertContentAt(1, 'First. ') })
    expect(undo.disabled).toBe(false)
    const settled = probe.panelRenders
    act(() => { invoices.commands.insertContentAt(1, 'Second. ') })
    act(() => { invoices.commands.insertContentAt(1, 'Third. ') })
    act(() => { billing.commands.insertContentAt(1, 'Elsewhere. ') })
    expect(probe.panelRenders).toBe(settled)
    expect(undo.disabled).toBe(false)
    fireEvent.click(undo)
    expect(screen.getByLabelText('Section 1.1 prose').textContent).not.toContain('Third. ')
    expect(screen.getByLabelText('Section 1 prose').textContent).toContain('Elsewhere. ')
  })

  it('retains workspace move history when the document view remounts', () => {
    const { doc, a, b, props } = setup()
    const history = createStructureHistory(doc)
    const first = render(<Plan {...props} structureHistory={history} />)
    act(() => { history.move(a, b, 'child') })
    first.unmount()
    render(<Plan {...props} structureHistory={history} />)
    fireEvent.click(screen.getByRole('button', { name: 'Undo section move' }))
    expect(readPlanTree(doc).find(n => n.id === a)?.parent).toBeNull()
    history.destroy()
  })

  it('changes focus mode without replacing the current editor or its collaborative content', () => {
    const { fragment, props } = setup()
    const view = render(<Plan {...props} />)
    const editor = screen.getByLabelText('Section 1.1 prose')
    view.rerender(<Plan {...props} focusMode />)
    expect(screen.getByLabelText('Section 1.1 prose')).toBe(editor)
    expect(view.container.querySelector('aside')?.style.display).toBe('none')
    act(() => { (fragment.get(0) as Y.XmlElement).insert(1, [new Y.XmlText(' Extra detail.')]) })
    expect(editor.textContent).toContain('Extra detail.')
    view.rerender(<Plan {...props} focusMode={false} />)
    expect(screen.getByLabelText('Section 1.1 prose')).toBe(editor)
    expect(view.container.querySelector('aside')?.style.display).toBe('')
  })
})

function titleCaret(title: HTMLElement, end: boolean) {
  title.focus()
  const range = document.createRange()
  range.selectNodeContents(title)
  range.collapse(!end)
  const selection = window.getSelection()!
  selection.removeAllRanges()
  selection.addRange(range)
}

it('writes continuously between titles and prose without losing text', async () => {
  const { doc, child, fragment, props } = setup()
  render(<Plan {...props} />)
  const title = screen.getAllByLabelText('Rename section')[1]!
  const nextTitle = screen.getAllByLabelText('Rename section')[2]!
  title.textContent = 'Invoice details'
  act(() => { titleCaret(title, true) })
  fireEvent.keyDown(title, { key: 'Enter' })
  await waitFor(() => expect(document.activeElement).toBe(screen.getByLabelText('Section 1.1 prose')))
  expect(readPlanTree(doc).find(node => node.id === child)!.title).toBe('Invoice details')
  const prose = probe.editors.get('Section 1.1 prose')!
  expect(prose.state.selection.from).toBe(1)
  fireEvent.keyDown(prose.view.dom, { key: 'Backspace' })
  expect(document.activeElement).toBe(title)
  const remainder = window.getSelection()!.getRangeAt(0).cloneRange()
  remainder.setEndAfter(title.lastChild!)
  expect(remainder.toString()).toBe('')
  expect(fragment.toString()).toContain('Payment deadline is thirty days.')
  fireEvent.keyDown(title, { key: 'ArrowDown' })
  await waitFor(() => expect(document.activeElement).toBe(prose.view.dom))
  act(() => { prose.commands.setTextSelection(prose.state.doc.content.size - 1) })
  fireEvent.keyDown(prose.view.dom, { key: 'ArrowDown' })
  expect(document.activeElement).toBe(nextTitle)
  expect(window.getSelection()!.anchorOffset).toBe(0)
  fireEvent.keyDown(nextTitle, { key: 'ArrowUp' })
  await waitFor(() => expect(document.activeElement).toBe(prose.view.dom))
  expect(prose.state.selection.from).toBe(prose.state.doc.content.size - 1)
  expect(fragment.toString()).toContain('Payment deadline is thirty days.')
})

it('mounts offscreen prose when a heading boundary requests it', async () => {
  vi.stubGlobal('IntersectionObserver', class { observe() {} unobserve() {} disconnect() {} })
  const { props } = setup()
  render(<Plan {...props} />)
  expect(screen.queryByLabelText('Section 1.1 prose')).toBeNull()
  const title = screen.getAllByLabelText('Rename section')[1]!
  act(() => { titleCaret(title, true) })
  fireEvent.keyDown(title, { key: 'ArrowDown' })
  await waitFor(() => expect(document.activeElement).toBe(screen.getByLabelText('Section 1.1 prose')))
  expect(probe.editors.get('Section 1.1 prose')!.state.selection.from).toBe(1)
})

it('formats the selected prose through the shared toolbar and copies the stable section link', async () => {
  const { child, props } = setup()
  const writeText = vi.fn(async (_text: string) => {})
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
  render(<Plan {...props} project="takomo" />)
  const prose = probe.editors.get('Section 1.1 prose')!
  fireEvent.pointerDown(prose.view.dom)
  act(() => { prose.commands.setTextSelection({ from: 1, to: 8 }) })
  const style = screen.getByRole('combobox', { name: 'Paragraph style' }) as HTMLSelectElement
  expect(style.value).toBe('paragraph')
  fireEvent.mouseDown(screen.getByRole('button', { name: 'Bold' }))
  fireEvent.click(screen.getByRole('button', { name: 'Bold' }))
  expect(prose.state.selection.from).toBe(1)
  expect(prose.state.selection.to).toBe(8)
  expect(prose.isActive('bold')).toBe(true)
  fireEvent.change(style, { target: { value: 'h2' } })
  expect(style.value).toBe('h2')
  expect(prose.state.doc.firstChild!.type.name).toBe('heading')
  expect(prose.state.doc.firstChild!.textContent).toBe('Payment deadline is thirty days.')
  fireEvent.click(screen.getAllByRole('button', { name: 'Copy section link' })[1]!)
  await waitFor(() => expect(writeText).toHaveBeenCalledWith(expect.stringContaining(`section=${child}`)))
  expect(writeText.mock.calls[0]![0]).toContain('/projects/takomo/specification?view=document')
})

it('disables stale prose tools while editing a title and restores them on returning to prose', async () => {
  const { fragment, props } = setup()
  render(<Plan {...props} />)
  const prose = probe.editors.get('Section 1.1 prose')!
  fireEvent.pointerDown(prose.view.dom)
  act(() => { prose.commands.focus(); prose.commands.setTextSelection({ from: 1, to: 8 }) })
  const style = screen.getByRole('combobox', { name: 'Paragraph style' }) as HTMLSelectElement
  const comment = screen.getByRole('button', { name: 'Add comment' }) as HTMLButtonElement
  await waitFor(() => expect(style.disabled).toBe(false))
  expect(comment.disabled).toBe(false)
  const title = screen.getAllByLabelText('Rename section')[1]!
  act(() => { titleCaret(title, true) })
  expect(style.disabled).toBe(true)
  expect(comment.disabled).toBe(true)
  fireEvent.keyDown(title, { key: 'Enter' })
  await waitFor(() => expect(document.activeElement).toBe(prose.view.dom))
  expect(style.disabled).toBe(false)
  // Enter places a collapsed caret at the beginning, so commenting waits for
  // an actual prose selection rather than retaining the old title-era range.
  expect(comment.disabled).toBe(true)
  act(() => { prose.commands.setTextSelection({ from: 1, to: 8 }) })
  expect(comment.disabled).toBe(false)
  expect(fragment.toString()).toContain('Payment deadline is thirty days.')
})
