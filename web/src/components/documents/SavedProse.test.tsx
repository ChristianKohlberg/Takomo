import { render, screen } from '@testing-library/react'
import { expect, it, vi } from 'vitest'
import { SavedProse } from './SavedProse'
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
