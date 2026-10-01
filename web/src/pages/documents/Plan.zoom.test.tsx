// Zoom scales the document column only: the page gets the factor, the
// toolbar, the outline and the pane around it never do.
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { Awareness } from 'y-protocols/awareness'
import * as Y from 'yjs'
import Plan, { type PlanProps } from './Plan'
import { createNode } from '@/lib/mindmap-crdt'

vi.mock('@/components/documents/DocumentHybridSearch', () => ({ DocumentHybridSearch: () => null }))
vi.mock('@/components/documents/DocumentEmbeddingStatus', () => ({ DocumentEmbeddingStatus: () => null }))
vi.mock('@/hooks/useEmbeddingStatus', () => ({
  EmbeddingStatusProvider: ({ children }: { children: React.ReactNode }) => children,
  useEmbeddingStatus: () => null,
}))

const fixtures: { doc: Y.Doc; awareness: Awareness }[] = []
beforeEach(() => {
  localStorage.clear()
  Element.prototype.scrollIntoView = vi.fn()
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList
  Range.prototype.getBoundingClientRect = () => new DOMRect()
  vi.stubGlobal('fetch', vi.fn(async () => new Response(JSON.stringify({ items: [] }))))
})
afterEach(() => {
  cleanup()
  vi.unstubAllGlobals()
  for (const { doc, awareness } of fixtures.splice(0)) { awareness.destroy(); doc.destroy() }
})

function props(project: string): PlanProps {
  const doc = new Y.Doc()
  const awareness = new Awareness(doc)
  fixtures.push({ doc, awareness })
  createNode(doc, { title: 'Billing', parent: null, by: 'Ada' })
  createNode(doc, { title: 'Reports', parent: null, by: 'Ada' })
  return {
    token: 'token', project,
    connection: { ydoc: doc, provider: { awareness } } as PlanProps['connection'],
    session: { object: 'mm', kind: 'mindmap', mindmap: 'mm', session: 'test', token: '', can_write: true, display: 'Ada', expires_at: '', url: '', room: 'mm' },
    testsFor: () => ({ total: 0, failing: 0 }), onShowTests: vi.fn(), testsLabel: 'Tests', failedLabel: 'failed', onError: vi.fn(),
    standing: {}, trace: new Map(), onReview: vi.fn(), onEdited: vi.fn(), onShowOnMap: vi.fn(), onDecided: vi.fn(), onSkipped: vi.fn(),
    labels: { readOnly: 'Read only', empty: 'Empty', emptyHint: '', proseEmpty: 'Write here', proseLabel: 'Section {n} prose' },
    railLabels: { outline: 'Outline', expand: 'Expand section', collapse: 'Collapse section', folded: '{n} sections inside', untitled: 'Untitled', standingConfirmed: 'Agreed', standingChanged: 'Changed', standingUnseen: 'Unread', pending: '{n} pending' },
    sectionLabels: { actions: 'Section actions', renameSection: 'Rename section', untitled: 'Untitled', standingConfirmed: 'Agreed', standingChanged: 'Changed', standingUnseen: 'Unread', review: 'Reviewed', reviewHint: 'Mark reviewed', showOnMap: 'Show on map', history: 'History', hideHistory: 'Hide history', historyEmpty: 'No history', historyMore: '{n} older', proposals: 'Proposals', hideProposals: 'Hide proposals', pendingBadge: '{n} waiting', needWrite: 'Read only', kinds: { authored: 'Written', renamed: 'Renamed', edited: 'Edited', moved: 'Moved', pruned: 'Removed', reviewed: 'Reviewed', proposed: 'Proposed', accepted: 'Accepted', rejected: 'Rejected' } },
    proposalLabels: { heading: 'Proposals', empty: 'No proposals', pending: 'Pending', accepted: 'Accepted', rejected: 'Rejected', accept: 'Accept', reject: 'Reject', by: 'By', partial: 'Partial', opReplace: 'Replace', opInsert: 'Insert', opDelete: 'Delete', readOnly: 'Read only' },
  }
}

const zoomOf = (element: Element | null) => (element as HTMLElement | null)?.style.getPropertyValue('--document-zoom') ?? ''

it('zooms the document page and nothing around it, per project', async () => {
  localStorage.setItem('takomo:document-zoom:alpha', '1.5')
  const view = render(<Plan {...props('alpha')} />)
  await waitFor(() => expect(document.querySelector('.document-page')).toBeTruthy())
  const page = document.querySelector('.document-page')!
  expect(zoomOf(page)).toBe('1.5')
  // The appearance variables still reach the page beside the zoom.
  expect((page as HTMLElement).getAttribute('style')).toContain('--document-zoom')
  const toolbar = screen.getByRole('toolbar', { name: 'Document tools' })
  expect(toolbar.contains(screen.getByRole('button', { name: 'Zoom: 150%' }))).toBe(true)
  // Only the page carries the factor: not the toolbar, the outline, the column or the pane.
  const zoomed = [...document.querySelectorAll<HTMLElement>('[style]')].filter(el => el.style.getPropertyValue('--document-zoom'))
  expect(zoomed).toEqual([page])
  expect(page.contains(toolbar)).toBe(false)
  expect(page.contains(document.getElementById('document-outline'))).toBe(false)

  act(() => { fireEvent.keyDown(window, { key: '0', code: 'Digit0', ctrlKey: true, altKey: true }) })
  expect(zoomOf(page)).toBe('1')
  expect(localStorage.getItem('takomo:document-zoom:alpha')).toBeNull()
  view.unmount()
  render(<Plan {...props('beta')} />)
  await waitFor(() => expect(zoomOf(document.querySelector('.document-page'))).toBe('1'))
})
