import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { SavedProse } from './SavedProse'
import type { SavedBlock } from '@/lib/saved-prose'
import type { SavedSection } from '@/lib/spec-history'
it('reads old XML as inert content, preserving literal text and table spans', () => {
  const node = { id: 'source', prose_xml: '<paragraph><bold>Important</bold> &lt;script&gt;literal&lt;/script&gt;</paragraph><table><tableRow><tableCell colspan="2"><paragraph>Joined cell</paragraph></tableCell></tableRow></table>' } as SavedSection
  const { container } = render(<SavedProse node={node} nodes={[node]} access={{ token: '', project: 'demo' }} missing="Missing section" />)
  expect(container.querySelector('strong')?.textContent).toBe('Important')
  expect(container.textContent).toContain('<script>literal</script>')
  expect(container.querySelector('script')).toBeNull()
  expect(container.querySelector('td')?.colSpan).toBe(2)
})
it('resolves snapshot reference titles and blocks unsafe link navigation', () => {
  const node = { id: 'source', prose_structure: [{ tag: 'paragraph', children: [{ text: [{ insert: 'Unsafe link', attributes: { link: { href: 'javascript:alert(1)' } } }] }, { tag: 'sectionReference', attributes: { sectionId: 'target' }, children: [{ text: [{ insert: 'Old title' }] }] }] }] } as SavedSection
  const target = { id: 'target', title: 'New title' } as SavedSection
  const { container, rerender } = render(<SavedProse node={node} nodes={[node, target]} access={{ token: '', project: 'demo' }} missing="Missing section" />)
  expect(screen.getByText('New title')).toBeTruthy()
  expect(container.querySelector('a')).toBeNull()
  rerender(<SavedProse node={node} nodes={[node]} access={{ token: '', project: 'demo' }} missing="Missing section" />)
  expect(screen.getByText('Old title (Missing section)')).toBeTruthy()
})


it('renders Yjs lowercase XML tables with valid table children and resolves references', () => {
  const error = vi.spyOn(console, 'error').mockImplementation(() => {})
  try {
    const node = { id: 'source', prose_xml: '<table>\n<tablerow>\n<tableheader colspan="2"><paragraph>Header</paragraph></tableheader>\n</tablerow><tablerow><tablecell><paragraph><sectionreference sectionId="target">Old</sectionreference></paragraph></tablecell></tablerow>\n</table><codeblock language="text">Plain code</codeblock>' } as SavedSection
    const target = { id: 'target', title: 'Current target' } as SavedSection
    const { container } = render(<SavedProse node={node} nodes={[node, target]} access={{ token: '', project: 'demo' }} missing="Missing section" />)
    expect(container.querySelectorAll('tbody > tr')).toHaveLength(2)
    expect(container.querySelector('tbody > span, tr > span')).toBeNull()
    expect(container.querySelector('th')?.colSpan).toBe(2)
    expect(screen.getByText('Current target')).toBeTruthy()
    expect(container.querySelector('pre')?.textContent).toBe('Plain code')
    expect(error).not.toHaveBeenCalled()
  } finally { error.mockRestore() }
})

it('renders saved collapsible tables closed, retaining summary and safe inline code', () => {
  const node = { id: 'source', prose_xml: '<collapsibleblock><collapsiblesummary>Permissions</collapsiblesummary><collapsiblecontent><table><tablerow><tablecell><paragraph><code>patron.update</code></paragraph></tablecell></tablerow></table></collapsiblecontent></collapsibleblock>' } as SavedSection
  const { container } = render(<SavedProse node={node} nodes={[node]} access={{ token: '', project: 'demo' }} missing="Missing section" />)
  expect(container.querySelector('details')?.open).toBe(false)
  expect(container.querySelector('summary')?.textContent).toBe('Permissions')
  expect(container.querySelector('td code')?.textContent).toBe('patron.update')
})


const cell = (text: string, tag = 'tableCell', rowspan?: number): SavedBlock => ({ tag, attributes: rowspan ? { rowspan } : {}, children: [{ tag: 'paragraph', children: [{ text: [{ insert: text }] }] }] })
const table = (body: number, rowspanAt?: { row: number; span: number }): SavedBlock => ({ tag: 'table', children: [
  { tag: 'tableRow', children: [cell('Name', 'tableHeader'), cell('Value', 'tableHeader')] },
  ...Array.from({ length: body }, (_, index) => {
    const row = index + 1
    const merged = rowspanAt && row > rowspanAt.row && row < rowspanAt.row + rowspanAt.span
    return { tag: 'tableRow', children: [...(merged ? [] : [cell(`r${row}`, 'tableCell', rowspanAt?.row === row ? rowspanAt.span : undefined)]), cell(`v${row}`)] }
  }),
] })
const node = (block: SavedBlock) => ({ id: 'n1', title: 'Section', prose_structure: [block] }) as unknown as SavedSection
const mount = (block: SavedBlock, locale: 'en' | 'de' = 'en') => render(<SavedProse node={node(block)} nodes={[]} access={{ token: '', project: '' }} missing="Missing" locale={locale} />)
const hidden = (container: HTMLElement) => container.querySelectorAll('tr.document-long-table-hidden').length

describe('SavedProse long tables (read-only projection)', () => {
  it('renders ten body rows whole and collapses eleven', () => {
    const ten = mount(table(10))
    expect(ten.container.querySelector('[data-long-table]')).toBeNull()
    expect(screen.queryByRole('button')).toBeNull()
    ten.unmount()
    const eleven = mount(table(11))
    expect(hidden(eleven.container)).toBe(1)
    const button = screen.getByRole('button', { name: 'Show all 11 rows' })
    expect(button.getAttribute('aria-expanded')).toBe('false')
    expect(document.getElementById(button.getAttribute('aria-controls')!)!.tagName).toBe('TABLE')
    expect(screen.getByText('Showing 10 of 11 rows')).toBeTruthy()
  })
  it('toggles, in German', () => {
    const view = mount(table(13), 'de')
    fireEvent.click(screen.getByRole('button', { name: 'Alle 13 Zeilen anzeigen' }))
    expect(hidden(view.container)).toBe(0)
    expect(screen.getByRole('button', { name: 'Weniger anzeigen' }).getAttribute('aria-expanded')).toBe('true')
    fireEvent.click(screen.getByRole('button', { name: 'Weniger anzeigen' }))
    expect(hidden(view.container)).toBe(3)
  })
  it('never cuts through a merged cell', () => {
    const view = mount(table(15, { row: 10, span: 3 }))
    // Rows 10–12 are merged, so rows 1–12 stay and 13–15 hide.
    expect(hidden(view.container)).toBe(3)
    expect(screen.getByText('Showing 12 of 15 rows')).toBeTruthy()
  })
})
