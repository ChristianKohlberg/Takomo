import { describe, it, expect } from 'vitest'
import { render, cleanup } from '@testing-library/react'
import { Markdown } from './Markdown'

describe('<Markdown>', () => {
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
