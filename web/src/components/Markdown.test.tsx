import { describe, it, expect, vi } from 'vitest'
import { render, cleanup } from '@testing-library/react'
import { Markdown } from './Markdown'

const { preview } = vi.hoisted(() => ({ preview: vi.fn(() => vi.fn()) }))
vi.mock('../lib/diagram', async (original) => ({ ...await original<typeof import('../lib/diagram')>(), mountDiagram: preview }))

describe('<Markdown>', () => {
  it('renders Mermaid fences with source retained and cancels replaced previews', () => {
    const { container, rerender, unmount } = render(<Markdown text={'```mermaid\nflowchart TD\nA --> B\n```'} />)
    expect(container.querySelector('code[data-lang="mermaid"]')?.textContent).toBe('flowchart TD\nA --> B')
    expect(preview).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'flowchart TD\nA --> B', 'mermaid', null)
    const cancel = preview.mock.results.at(-1)!.value
    rerender(<Markdown text="No diagram" />)
    expect(cancel).toHaveBeenCalled()
    expect(container.querySelector('code')).toBeNull()
    unmount()
  })

  it.each(['plantuml', 'puml', 'salt', 'd2'])('renders %s fences using the object project', (language) => {
    localStorage.clear()
    const access = { token: 'reader', project: 'object-project' }
    const { unmount } = render(<Markdown text={'```' + language + '\nsource\n```'} diagramAccess={access} />)
    expect(preview).toHaveBeenLastCalledWith(expect.any(HTMLElement), 'source', language === 'd2' ? 'd2' : 'plantuml', access)
    unmount()
  })

  it('mounts the rendered tree', () => {
    const { container } = render(<Markdown text="**bold** and `code`" />)
    expect(container.querySelector('.md b')?.textContent).toBe('bold')
    expect(container.querySelector('.md code')?.textContent).toBe('code')
    cleanup()
  })

  it('never injects markup from the source', () => {
    const { container } = render(<Markdown text="<img src=x onerror=alert(1)>" />)
    expect(container.querySelector('img')).toBeNull()
    expect(container.textContent).toContain('<img src=x onerror=alert(1)>')
    cleanup()
  })

  it('re-renders when the text changes', () => {
    const { container, rerender } = render(<Markdown text="one" />)
    expect(container.textContent).toBe('one')
    rerender(<Markdown text="two" />)
    expect(container.textContent).toBe('two')
    // Replaced, not appended.
    expect(container.querySelectorAll('.md')).toHaveLength(1)
    cleanup()
  })

  it('takes a variant class and a host class', () => {
    const { container } = render(<Markdown text="x" variant="compact" className="host" />)
    expect(container.querySelector('.host')).not.toBeNull()
    expect(container.querySelector('.md.compact')).not.toBeNull()
    cleanup()
  })

  it('renders empty for null text', () => {
    const { container } = render(<Markdown text={null} />)
    expect(container.textContent).toBe('')
    cleanup()
  })
})
