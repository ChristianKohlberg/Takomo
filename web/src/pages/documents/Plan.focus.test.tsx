// Section focus (`focus=`): the document narrowed to one section's subtree.
// Driven through the real URL hooks under a MemoryRouter, so history entries,
// Back and the dropped-param fallback are the ones the app uses.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { MemoryRouter, Route, Routes, useLocation, useNavigate } from 'react-router'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { Awareness } from 'y-protocols/awareness'
import * as Y from 'yjs'
import type { Editor } from '@tiptap/react'
import Plan, { type PlanProps } from './Plan'
import { useWorkspaceFocus, useWorkspaceSection } from '@/hooks/useWorkspaceSection'
import { createNode } from '@/lib/mindmap-crdt'
import { PROPOSALS_KEY } from '@/lib/plan-proposals'
import { createCommentThread } from '@/lib/document-comments'
import type { SearchResult } from '@/lib/hybrid-search'

const probe = vi.hoisted(() => ({
  editors: new Map<string, Editor>(),
  mounted: new Set<string>(),
  insert: new Map<string, (level: 1 | 2 | 3, title: string) => boolean>(),
  search: null as null | { scope: { ids: ReadonlySet<string> } | null | undefined; onNavigate: (result: SearchResult) => void },
  location: '',
  back: () => {},
  navigate: (_search: string) => {},
}))
vi.mock('./SectionEditor', async (importOriginal) => {
  const mod = await importOriginal<typeof import('./SectionEditor')>()
  const Original = mod.default
  const Probed: typeof Original = (props) => { const id = props.sectionId ?? ''; return <Original {...props} onEditor={(editor) => {
    if (props.onInsertSection) probe.insert.set(id, props.onInsertSection)
    if (editor) { probe.editors.set(props.label, editor); probe.mounted.add(id) }
    else { probe.editors.delete(props.label); probe.mounted.delete(id) }
    props.onEditor?.(editor)
  }} /> }
  return { ...mod, default: Probed }
})
// The discovery search talks to the server; the page's contract with it is the
// scope it hands over and what it does with a chosen hit.
vi.mock('@/components/documents/DocumentHybridSearch', () => ({
  DocumentHybridSearch: (props: { scope?: { ids: ReadonlySet<string> } | null; onNavigate: (result: SearchResult) => void }) => {
    probe.search = { scope: props.scope, onNavigate: props.onNavigate }
    return null
  },
}))
vi.mock('@/components/documents/DocumentEmbeddingStatus', () => ({ DocumentEmbeddingStatus: () => null }))
vi.mock('@/hooks/useEmbeddingStatus', () => ({
  EmbeddingStatusProvider: ({ children }: { children: React.ReactNode }) => children,
  useEmbeddingStatus: () => null,
}))

const fixtures: { doc: Y.Doc; awareness: Awareness }[] = []
beforeEach(() => {
  localStorage.clear()
  probe.editors.clear()
  probe.mounted.clear()
  probe.insert.clear()
  probe.search = null
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

/** 1 Billing › 1.1 Invoices › 1.1.1 Terms, and 2 Reports beside it. */
function setup(canWrite = true) {
  const doc = new Y.Doc()
  const awareness = new Awareness(doc)
  fixtures.push({ doc, awareness })
  const billing = createNode(doc, { title: 'Billing', parent: null, by: 'Ada' })!
  const invoices = createNode(doc, { title: 'Invoices', parent: billing, by: 'Ada' })!
  const terms = createNode(doc, { title: 'Terms', parent: invoices, by: 'Ada' })!
  const reports = createNode(doc, { title: 'Reports', parent: null, by: 'Ada' })!
  const props: PlanProps = {
    token: 'token',
    project: 'p',
    connection: { ydoc: doc, provider: { awareness } } as PlanProps['connection'],
    session: { object: 'mm-focus', kind: 'mindmap', mindmap: 'mm-focus', session: 'test', token: '', can_write: canWrite, display: 'Ada', expires_at: '', url: '', room: 'mm-focus' },
    testsFor: () => ({ total: 0, failing: 0 }), onShowTests: vi.fn(), testsLabel: 'Tests', failedLabel: 'failed', onError: vi.fn(),
    standing: {}, trace: new Map(), onReview: vi.fn(), onEdited: vi.fn(), onShowOnMap: vi.fn(), onDecided: vi.fn(), onSkipped: vi.fn(),
    labels: { readOnly: 'Read only', empty: 'Empty', emptyHint: '', proseEmpty: 'Write here', proseLabel: 'Section {n} prose' },
    railLabels: { outline: 'Outline', expand: 'Expand section', collapse: 'Collapse section', folded: '{n} sections inside', untitled: 'Untitled', standingConfirmed: 'Agreed', standingChanged: 'Changed', standingUnseen: 'Unread', pending: '{n} pending' },
    sectionLabels: { actions: 'Section actions', renameSection: 'Rename section', untitled: 'Untitled', standingConfirmed: 'Agreed', standingChanged: 'Changed', standingUnseen: 'Unread', review: 'Reviewed', reviewHint: 'Mark reviewed', showOnMap: 'Show on map', history: 'History', hideHistory: 'Hide history', historyEmpty: 'No history', historyMore: '{n} older', proposals: 'Proposals', hideProposals: 'Hide proposals', pendingBadge: '{n} waiting', needWrite: 'Read only', kinds: { authored: 'Written', renamed: 'Renamed', edited: 'Edited', moved: 'Moved', pruned: 'Removed', reviewed: 'Reviewed', proposed: 'Proposed', accepted: 'Accepted', rejected: 'Rejected' } },
    proposalLabels: { heading: 'Proposals', empty: 'No proposals', pending: 'Pending', accepted: 'Accepted', rejected: 'Rejected', accept: 'Accept', reject: 'Reject', by: 'By', partial: 'Partial', opReplace: 'Replace', opInsert: 'Insert', opDelete: 'Delete', readOnly: 'Read only' },
  }
  return { doc, billing, invoices, terms, reports, props }
}

function Harness({ props }: { props: PlanProps }) {
  const [section, select] = useWorkspaceSection()
  const [focus, setFocus] = useWorkspaceFocus()
  const location = useLocation()
  const navigate = useNavigate()
  probe.location = location.search
  probe.back = () => { void navigate(-1) }
  probe.navigate = search => { void navigate(`/projects/p/specification${search}`) }
  return <Plan {...props} focusSection={section} onSelection={select} sectionFocus={focus} onSectionFocus={setFocus} />
}

function open(props: PlanProps, search = '?view=document') {
  return render(
    <MemoryRouter initialEntries={[`/projects/p/specification${search}`]}>
      <Routes><Route path="*" element={<Harness props={props} />} /></Routes>
    </MemoryRouter>,
  )
}

const params = () => new URLSearchParams(probe.location)
const band = () => screen.queryByRole('region', { name: 'Section focus' })
const mountedTitles = (ids: Record<string, string>) => Object.entries(ids).filter(([, id]) => probe.mounted.has(id)).map(([title]) => title).sort()

describe('section focus', () => {
  it('enters from the section actions menu and mounts only the focused subtree', async () => {
    const { billing, invoices, terms, reports, props } = setup()
    open(props)
    await waitFor(() => expect(probe.mounted.size).toBe(4))
    const billingSection = document.querySelector<HTMLElement>('.document-section')!
    fireEvent.click(within(billingSection).getAllByRole('button', { name: 'Section actions' })[0]!)
    fireEvent.click(within(billingSection).getByRole('button', { name: 'Show only this section' }))

    await waitFor(() => expect(params().get('focus')).toBe(billing))
    expect(params().get('section')).toBe(billing)
    expect(band()).toBeTruthy()
    expect(within(band()!).getByText('Focus: 1 Billing')).toBeTruthy()
    // Reports' editor is never constructed while Billing is focused.
    await waitFor(() => expect(mountedTitles({ billing, invoices, terms, reports })).toEqual(['billing', 'invoices', 'terms']))
    expect(screen.queryByText('Reports')).toBeNull()
    // Keyboard focus lands on the focused section's heading.
    await waitFor(() => expect(document.activeElement?.textContent).toBe('Billing'))
  })

  it('keeps real section numbers and narrows the outline to the subtree', async () => {
    const { invoices, props } = setup()
    open(props, `?view=document&focus=${invoices}`)
    await waitFor(() => expect(band()).toBeTruthy())
    const outline = screen.getByRole('tree', { name: 'Outline' })
    expect(within(outline).getAllByRole('treeitem').map(item => item.getAttribute('aria-label'))).toEqual(['1.1 Invoices', '1.1.1 Terms'])
    const numbers = [...document.querySelectorAll('.document-section-number')].map(node => node.textContent)
    expect(numbers).toEqual(['1.1', '1.1.1'])
  })

  it('enters from the outline context menu (Shift+F10)', async () => {
    const { reports, props } = setup()
    open(props)
    const row = await screen.findByRole('treeitem', { name: '2 Reports' })
    fireEvent.keyDown(row, { key: 'F10', shiftKey: true })
    const menu = screen.getByRole('menu')
    expect(within(menu).getByRole('menuitem', { name: 'Move section…' })).toBeTruthy()
    fireEvent.click(within(menu).getByRole('menuitem', { name: 'Show only this section' }))
    await waitFor(() => expect(params().get('focus')).toBe(reports))
    expect(screen.queryByRole('menu')).toBeNull()
    expect(within(screen.getByRole('tree', { name: 'Outline' })).getAllByRole('treeitem')).toHaveLength(1)
  })

  it('offers the outline menu to readers with only the focus entry', async () => {
    const { props } = setup(false)
    open(props)
    const row = await screen.findByRole('treeitem', { name: '2 Reports' })
    fireEvent.contextMenu(row)
    const menu = screen.getByRole('menu')
    expect(within(menu).getAllByRole('menuitem').map(item => item.textContent)).toEqual(['Show only this section'])
    fireEvent.keyDown(menu, { key: 'Escape' })
    expect(screen.queryByRole('menu')).toBeNull()
    expect(params().get('focus')).toBeNull()
  })

  it('toggles focus on the current section with Alt+F', async () => {
    const { invoices, props } = setup()
    open(props)
    const row = await screen.findByRole('treeitem', { name: '1.1 Invoices' })
    fireEvent.click(within(row).getByRole('button', { name: '1.1 Invoices' }))
    await waitFor(() => expect(params().get('section')).toBe(invoices))
    fireEvent.keyDown(window, { key: 'ƒ', code: 'KeyF', altKey: true })
    await waitFor(() => expect(params().get('focus')).toBe(invoices))
    fireEvent.keyDown(window, { key: 'f', code: 'KeyF', altKey: true })
    await waitFor(() => expect(params().get('focus')).toBeNull())
    // Alt+Shift+F is the same command for browsers that keep Alt+F for their menu.
    fireEvent.keyDown(window, { key: 'F', code: 'KeyF', altKey: true, shiftKey: true })
    await waitFor(() => expect(params().get('focus')).toBe(invoices))
  })

  it('navigates by breadcrumb: an ancestor focuses it, the root leaves focus', async () => {
    const { billing, invoices, terms, props } = setup()
    open(props, `?view=document&focus=${terms}`)
    const crumbs = await screen.findByRole('navigation', { name: 'Position in the specification' })
    expect(crumbs.textContent).toBe('Specification›1 Billing›1.1 Invoices›1.1.1 Terms')
    expect(within(crumbs).getByText('1.1.1 Terms').getAttribute('aria-current')).toBe('page')
    fireEvent.click(within(crumbs).getByRole('button', { name: '1.1 Invoices' }))
    await waitFor(() => expect(params().get('focus')).toBe(invoices))
    fireEvent.click(within(screen.getByRole('navigation', { name: 'Position in the specification' })).getByRole('button', { name: '1 Billing' }))
    await waitFor(() => expect(params().get('focus')).toBe(billing))
    fireEvent.click(within(screen.getByRole('navigation', { name: 'Position in the specification' })).getByRole('button', { name: 'Specification' }))
    await waitFor(() => expect(params().get('focus')).toBeNull())
    expect(band()).toBeNull()
  })

  it('leaves focus with the band button, Escape and browser Back', async () => {
    const { billing, reports, props } = setup()
    open(props)
    const reportsRow = await screen.findByRole('treeitem', { name: '2 Reports' })
    fireEvent.click(within(reportsRow).getByRole('button', { name: '2 Reports' }))
    await waitFor(() => expect(params().get('section')).toBe(reports))

    // Entered from the outline while Reports was current: exit returns to Reports.
    fireEvent.contextMenu(await screen.findByRole('treeitem', { name: '1 Billing' }))
    fireEvent.click(screen.getByRole('menuitem', { name: 'Show only this section' }))
    await waitFor(() => expect(params().get('focus')).toBe(billing))
    const exit = within(band()!).getByRole('button', { name: 'Show all sections' })
    expect(exit.tagName).toBe('BUTTON')
    fireEvent.click(exit)
    await waitFor(() => expect(params().get('focus')).toBeNull())
    expect(params().get('section')).toBe(reports)
    await waitFor(() => expect(document.activeElement?.textContent).toBe('Reports'))

    // Escape, from somewhere that does not claim it.
    fireEvent.contextMenu(screen.getByRole('treeitem', { name: '1 Billing' }))
    fireEvent.click(screen.getByRole('menuitem', { name: 'Show only this section' }))
    await waitFor(() => expect(params().get('focus')).toBe(billing))
    ;(document.activeElement as HTMLElement | null)?.blur()
    fireEvent.keyDown(window, { key: 'Escape' })
    await waitFor(() => expect(params().get('focus')).toBeNull())

    // Back returns to the focused view, and Back again to the view before it.
    act(() => probe.back())
    await waitFor(() => expect(params().get('focus')).toBe(billing))
    expect(band()).toBeTruthy()
    act(() => probe.back())
    await waitFor(() => expect(params().get('focus')).toBeNull())
  })

  it('does not leave focus on an Escape a menu consumed', async () => {
    const { billing, props } = setup()
    open(props, `?view=document&focus=${billing}`)
    await waitFor(() => expect(band()).toBeTruthy())
    const section = document.querySelector<HTMLElement>('.document-section')!
    const trigger = within(section).getAllByRole('button', { name: 'Section actions' })[0]!
    fireEvent.click(trigger)
    fireEvent.keyDown(trigger, { key: 'Escape' })
    expect(params().get('focus')).toBe(billing)
    // A title being edited owns Escape too.
    const title = section.querySelector<HTMLElement>('.document-heading')!
    title.focus()
    fireEvent.keyDown(title, { key: 'Escape' })
    expect(params().get('focus')).toBe(billing)
  })

  it('scopes discovery search to the focus and leaves focus for a hit outside it', async () => {
    const { billing, invoices, terms, reports, props } = setup()
    open(props, `?view=document&focus=${billing}`)
    await waitFor(() => expect(band()).toBeTruthy())
    expect([...(probe.search!.scope!.ids)].sort()).toEqual([billing, invoices, terms].sort())
    const hit: SearchResult = { node_id: terms, title: 'Terms', heading_path: [], excerpt: '', passage: '', highlights: [], match_kind: 'keyword' }
    act(() => probe.search!.onNavigate(hit))
    await waitFor(() => expect(params().get('section')).toBe(terms))
    expect(params().get('focus')).toBe(billing)
    act(() => probe.search!.onNavigate({ ...hit, node_id: reports, title: 'Reports' }))
    await waitFor(() => expect(params().get('focus')).toBeNull())
    expect(params().get('section')).toBe(reports)
    expect(probe.search!.scope).toBeNull()
  })

  it('leaves focus when a navigation elsewhere changes the selection, but not on a cold link', async () => {
    const { billing, reports, props } = setup()
    // A shared link whose selection is outside its focus keeps the focus.
    const view = open(props, `?view=document&focus=${billing}&section=${reports}`)
    await waitFor(() => expect(band()).toBeTruthy())
    expect(params().get('focus')).toBe(billing)
    view.unmount()
    // An agent's link to a section outside the focus leaves it and goes there.
    function Agent() {
      const [, select] = useWorkspaceSection()
      return <button type="button" onClick={() => select(reports)}>agent link</button>
    }
    render(
      <MemoryRouter initialEntries={[`/projects/p/specification?view=document&focus=${billing}`]}>
        <Routes><Route path="*" element={<><Agent /><Harness props={props} /></>} /></Routes>
      </MemoryRouter>,
    )
    await waitFor(() => expect(band()).toBeTruthy())
    fireEvent.click(screen.getByRole('button', { name: 'agent link' }))
    await waitFor(() => expect(params().get('focus')).toBeNull())
    expect(params().get('section')).toBe(reports)
  })

  it('leaves focus to show a comment thread outside it', async () => {
    const { doc, billing, reports, props } = setup()
    createCommentThread(doc, reports, { quote: 'Reports', start: {}, end: {} }, 'Ada', 'Check this')
    open(props, `?view=document&focus=${billing}`)
    await waitFor(() => expect(band()).toBeTruthy())
    expect(within(band()!).getByText('1 open comment outside')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'All comments' }))
    fireEvent.click(await screen.findByRole('button', { name: 'Go to text' }))
    await waitFor(() => expect(params().get('focus')).toBeNull())
    expect(params().get('section')).toBe(reports)
  })

  it('counts pending proposals and open comment threads outside the focus', async () => {
    const { doc, billing, invoices, reports, props } = setup()
    const proposals = doc.getMap<string>(PROPOSALS_KEY)
    const pending = (id: string, node: string, status = 'pending') => proposals.set(id, JSON.stringify({ id, node, status, ops: [], by: 'agent', at: 0 }))
    pending('p1', reports); pending('p2', reports); pending('p3', invoices); pending('p4', reports, 'accepted')
    createCommentThread(doc, reports, { quote: 'x', start: {}, end: {} }, 'Ada', 'one')
    createCommentThread(doc, reports, { quote: 'y', start: {}, end: {} }, 'Ada', 'two')
    open(props, `?view=document&focus=${billing}`)
    await waitFor(() => expect(band()).toBeTruthy())
    expect(within(band()!).getByText('2 proposals outside · 2 open comments outside')).toBeTruthy()
  })

  it('falls back to the whole document with a notice for an unknown focus id', async () => {
    const { props } = setup()
    open(props, '?view=document&focus=mn-gone')
    await waitFor(() => expect(params().get('focus')).toBeNull())
    expect(screen.getByText(/focused section does not exist/)).toBeTruthy()
    expect(band()).toBeNull()
    await waitFor(() => expect(probe.mounted.size).toBe(4))
  })

  it('keeps a section created inside the focus and follows one created outside it', async () => {
    const { billing, props } = setup()
    open(props, `?view=document&focus=${billing}`)
    await waitFor(() => expect(band()).toBeTruthy())
    await waitFor(() => expect(probe.insert.has(billing)).toBe(true))
    act(() => { probe.insert.get(billing)!(2, 'Refunds') })
    await waitFor(() => expect(within(screen.getByRole('tree', { name: 'Outline' })).getByRole('treeitem', { name: /Refunds/ })).toBeTruthy())
    expect(params().get('focus')).toBe(billing)
    // An H1 is a new top-level section: focus is left and the reader follows it.
    act(() => { probe.insert.get(billing)!(1, 'Taxes') })
    await waitFor(() => expect(params().get('focus')).toBeNull())
    const taxes = screen.getByRole('treeitem', { name: /Taxes/ })
    expect(params().get('section')).toBeTruthy()
    expect(taxes.getAttribute('aria-selected')).toBe('true')
  })

  it('is view state only: the shared document never changes', async () => {
    const { doc, billing, invoices, props } = setup()
    open(props)
    await waitFor(() => expect(probe.mounted.size).toBe(4))
    const before = Y.encodeStateVector(doc)
    const updates = vi.fn()
    doc.on('update', updates)
    fireEvent.contextMenu(screen.getByRole('treeitem', { name: '1 Billing' }))
    fireEvent.click(screen.getByRole('menuitem', { name: 'Show only this section' }))
    await waitFor(() => expect(params().get('focus')).toBe(billing))
    fireEvent.click(within(screen.getByRole('navigation', { name: 'Position in the specification' })).getByRole('button', { name: 'Specification' }))
    await waitFor(() => expect(params().get('focus')).toBeNull())
    fireEvent.contextMenu(screen.getByRole('treeitem', { name: '1.1 Invoices' }))
    fireEvent.click(screen.getByRole('menuitem', { name: 'Show only this section' }))
    await waitFor(() => expect(params().get('focus')).toBe(invoices))
    act(() => probe.back())
    await waitFor(() => expect(params().get('focus')).toBeNull())
    doc.off('update', updates)
    expect(updates).not.toHaveBeenCalled()
    expect(Y.encodeStateVector(doc)).toEqual(before)
  })
})

// jsdom has no layout. This one stacks the sections at the heights the test
// sets, inside a column 600 px tall, so a section's position depends on every
// section above it — which is exactly what changes while editors mount. It is
// installed on the prototypes, so it is in place before the first render.
function fakeLayout(heights: Map<string, number>, zoom = 1) {
  // `heights` are page pixels; under the reader's CSS zoom the column sees them scaled.
  const heightOf = (section: HTMLElement) => (heights.get(section.dataset.section ?? '') ?? 200) * zoom
  const state = { scrollTop: 0 }
  const isColumn = (element: Element) => !!element.firstElementChild?.classList.contains('document-page') ||
    !!element.querySelector(':scope > .document-page')
  const sections = () => [...document.querySelectorAll<HTMLElement>('.document-section')]
  const total = () => sections().reduce((sum, section) => sum + heightOf(section), 0)
  const originalTop = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollTop')!
  const originalHeight = Object.getOwnPropertyDescriptor(Element.prototype, 'scrollHeight')!
  const originalClient = Object.getOwnPropertyDescriptor(Element.prototype, 'clientHeight')!
  Object.defineProperty(Element.prototype, 'scrollTop', {
    configurable: true,
    get(this: Element) { return isColumn(this) ? state.scrollTop : originalTop.get!.call(this) },
    set(this: Element, value: number) {
      if (isColumn(this)) state.scrollTop = Math.max(0, Math.min(value, Math.max(0, total() - 600)))
      else originalTop.set!.call(this, value)
    },
  })
  Object.defineProperty(Element.prototype, 'scrollHeight', { configurable: true, get(this: Element) { return isColumn(this) ? total() : originalHeight.get!.call(this) } })
  Object.defineProperty(Element.prototype, 'clientHeight', { configurable: true, get(this: Element) { return isColumn(this) ? 600 : originalClient.get!.call(this) } })
  restoreLayout = () => {
    Object.defineProperty(Element.prototype, 'scrollTop', originalTop)
    Object.defineProperty(Element.prototype, 'scrollHeight', originalHeight)
    Object.defineProperty(Element.prototype, 'clientHeight', originalClient)
  }
  const original = HTMLElement.prototype.getBoundingClientRect
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (this: HTMLElement) {
    if (isColumn(this)) return new DOMRect(0, 50, 800, 600)
    if (this.classList.contains('document-section')) {
      let top = 0
      for (const section of sections()) {
        const height = heightOf(section)
        if (section === this) return new DOMRect(0, 50 + top - state.scrollTop, 800, height)
        top += height
      }
    }
    return original.call(this)
  })
  /** The section's top relative to the column's top. */
  const offsetOf = (id: string) => document.querySelector<HTMLElement>(`[data-section="${id}"]`)!.getBoundingClientRect().top - 50
  return { offsetOf }
}
let restoreLayout = () => {}
const pause = (ms: number) => act(async () => { await new Promise(resolve => setTimeout(resolve, ms)) })

describe('navigating to a section', () => {
  afterEach(() => { vi.restoreAllMocks(); restoreLayout(); restoreLayout = () => {} })

  it('jumps at once and keeps the target at the top while the sections above it grow', async () => {
    const { billing, invoices, terms, reports, props } = setup()
    const heights = new Map([[billing, 900], [invoices, 1200], [terms, 400], [reports, 3000]])
    open(props)
    const row = await screen.findByRole('treeitem', { name: '2 Reports' })
    const { offsetOf } = fakeLayout(heights)
    fireEvent.click(within(row).getByRole('button', { name: '2 Reports' }))
    await waitFor(() => expect(offsetOf(reports)).toBe(0))
    // Editors above mount and grow after the jump (a table, a diagram).
    heights.set(invoices, 2600)
    await pause(50)
    heights.set(billing, 1500)
    heights.set(terms, 1100)
    await waitFor(() => expect(offsetOf(reports)).toBe(0))
    await pause(100)
    expect(offsetOf(reports)).toBe(0)
  })

  it.each([0.5, 1.5])('pins the heading at the top on the first outline click at %s zoom', async (zoom) => {
    localStorage.setItem('takomo:document-zoom:p', String(zoom))
    const { billing, invoices, terms, reports, props } = setup()
    const heights = new Map([[billing, 900], [invoices, 1200], [terms, 400], [reports, 3000]])
    open(props)
    const row = await screen.findByRole('treeitem', { name: '2 Reports' })
    expect(document.querySelector<HTMLElement>('.document-page')!.style.getPropertyValue('--document-zoom')).toBe(String(zoom))
    const { offsetOf } = fakeLayout(heights, zoom)
    fireEvent.click(within(row).getByRole('button', { name: '2 Reports' }))
    await waitFor(() => expect(offsetOf(reports)).toBe(0))
    heights.set(invoices, 2600)
    heights.set(billing, 1500)
    await waitFor(() => expect(offsetOf(reports)).toBe(0))
    await pause(100)
    expect(offsetOf(reports)).toBe(0)
  })

  it('does the same inside a section focus', async () => {
    const { billing, invoices, terms, props } = setup()
    const heights = new Map([[billing, 700], [invoices, 1000], [terms, 2500]])
    open(props, `?view=document&focus=${billing}`)
    await waitFor(() => expect(band()).toBeTruthy())
    const row = await screen.findByRole('treeitem', { name: '1.1.1 Terms' })
    const { offsetOf } = fakeLayout(heights)
    fireEvent.click(within(row).getByRole('button', { name: '1.1.1 Terms' }))
    await waitFor(() => expect(offsetOf(terms)).toBe(0))
    heights.set(invoices, 2200)
    heights.set(billing, 1300)
    await waitFor(() => expect(offsetOf(terms)).toBe(0))
    expect(params().get('focus')).toBe(billing)
  })

  it('stops holding the target once the reader scrolls', async () => {
    const { billing, invoices, terms, reports, props } = setup()
    const heights = new Map([[billing, 900], [invoices, 1200], [terms, 400], [reports, 3000]])
    open(props)
    const row = await screen.findByRole('treeitem', { name: '2 Reports' })
    const { offsetOf } = fakeLayout(heights)
    fireEvent.click(within(row).getByRole('button', { name: '2 Reports' }))
    await waitFor(() => expect(offsetOf(reports)).toBe(0))
    const column = document.querySelector<HTMLElement>('.document-page')!.parentElement!
    fireEvent.wheel(column)
    heights.set(billing, 1400)
    await pause(80)
    expect(offsetOf(reports)).toBe(500)
  })

  it('lands on a section opened cold from a link (section=) and holds it while the page fills in', async () => {
    const { billing, invoices, terms, reports, props } = setup()
    const heights = new Map([[billing, 900], [invoices, 1200], [terms, 400], [reports, 3000]])
    const { offsetOf } = fakeLayout(heights)
    open(props, `?view=document&section=${reports}`)
    await screen.findByRole('treeitem', { name: '2 Reports' })
    await waitFor(() => expect(offsetOf(reports)).toBe(0))
    heights.set(billing, 2000)
    heights.set(invoices, 150)
    await waitFor(() => expect(offsetOf(reports)).toBe(0))
    await pause(100)
    expect(offsetOf(reports)).toBe(0)
  })

  it('lands on a section when section= changes while the document is open (in-app navigation)', async () => {
    const { billing, invoices, terms, reports, props } = setup()
    const heights = new Map([[billing, 900], [invoices, 1200], [terms, 400], [reports, 3000]])
    const { offsetOf } = fakeLayout(heights)
    open(props)
    await screen.findByRole('treeitem', { name: '2 Reports' })
    act(() => probe.navigate(`?view=document&section=${terms}`))
    await waitFor(() => expect(offsetOf(terms)).toBe(0))
    heights.set(invoices, 2500)
    await waitFor(() => expect(offsetOf(terms)).toBe(0))
    // And back up the document to a section above.
    act(() => probe.navigate(`?view=document&section=${invoices}`))
    await waitFor(() => expect(offsetOf(invoices)).toBe(0))
    heights.set(billing, 1700)
    await waitFor(() => expect(offsetOf(invoices)).toBe(0))
    await pause(100)
    expect(offsetOf(invoices)).toBe(0)
  })
})
