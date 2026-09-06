import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { EditableText } from './EditableText'
function select(element: HTMLElement, from: number, to = from) {
  element.focus()
  const range = document.createRange()
  range.setStart(element.firstChild!, from)
  range.setEnd(element.firstChild!, to)
  const selection = window.getSelection()!
  selection.removeAllRanges()
  selection.addRange(range)
}
describe('EditableText writing navigation', () => {
  it('only crosses at title boundaries, leaving text intact', () => {
    const up = vi.fn(() => true), down = vi.fn(() => true)
    render(<EditableText value="Title" editable onCommit={async () => {}} onArrowUp={up} onArrowDown={down} aria-label="Title" />)
    const title = screen.getByLabelText('Title')
    select(title, 2)
    expect(fireEvent.keyDown(title, { key: 'ArrowUp' })).toBe(true)
    expect(fireEvent.keyDown(title, { key: 'ArrowDown' })).toBe(true)
    expect(up).not.toHaveBeenCalled()
    expect(down).not.toHaveBeenCalled()
    select(title, 0)
    expect(fireEvent.keyDown(title, { key: 'ArrowUp' })).toBe(false)
    select(title, 5)
    expect(fireEvent.keyDown(title, { key: 'ArrowDown' })).toBe(false)
    expect(up).toHaveBeenCalledOnce()
    expect(down).toHaveBeenCalledOnce()
    expect(title.textContent).toBe('Title')
  })
  it('leaves modified keys, composition, selections and unavailable destinations native for navigation', () => {
    const down = vi.fn(() => false)
    render(<EditableText value="Title" editable onCommit={async () => {}} onArrowDown={down} aria-label="Title" />)
    const title = screen.getByLabelText('Title')
    select(title, 5)
    for (const modifier of ['shiftKey', 'altKey', 'ctrlKey', 'metaKey', 'isComposing']) {
      expect(fireEvent.keyDown(title, { key: 'ArrowDown', [modifier]: true })).toBe(true)
    }
    select(title, 0, 5)
    expect(fireEvent.keyDown(title, { key: 'ArrowDown' })).toBe(true)
    expect(down).not.toHaveBeenCalled()
    select(title, 5)
    expect(fireEvent.keyDown(title, { key: 'ArrowDown' })).toBe(true)
    expect(down).toHaveBeenCalledOnce()
    expect(title.textContent).toBe('Title')
  })
  it('intercepts Enter with modifiers or a selection so a title stays one line, deferring only to composition', () => {
    const commit = vi.fn(async () => {}), enter = vi.fn()
    render(<EditableText value="Title" editable onCommit={commit} onEnter={enter} aria-label="Title" />)
    const title = screen.getByLabelText('Title')
    title.textContent = 'New title'
    select(title, 0, 3)
    expect(fireEvent.keyDown(title, { key: 'Enter', shiftKey: true })).toBe(false)
    expect(commit).toHaveBeenCalledWith('New title')
    expect(enter).toHaveBeenCalledOnce()
    expect(title.textContent).toBe('New title')
    expect(title.querySelector('br, div')).toBeNull()
    for (const modifier of ['altKey', 'ctrlKey', 'metaKey']) {
      select(title, 2, 5)
      expect(fireEvent.keyDown(title, { key: 'Enter', [modifier]: true })).toBe(false)
    }
    expect(enter).toHaveBeenCalledTimes(4)
    select(title, 9)
    expect(fireEvent.keyDown(title, { key: 'Enter', isComposing: true })).toBe(true)
    expect(enter).toHaveBeenCalledTimes(4)
    expect(title.textContent).toBe('New title')
  })
  it('commits a title before Enter moves into prose', () => {
    const commit = vi.fn(async () => {}), enter = vi.fn()
    render(<EditableText value="Title" editable onCommit={commit} onEnter={enter} aria-label="Title" />)
    const title = screen.getByLabelText('Title')
    title.textContent = 'New title'
    select(title, 9)
    fireEvent.keyDown(title, { key: 'Enter' })
    expect(commit).toHaveBeenCalledWith('New title')
    expect(enter).toHaveBeenCalledOnce()
  })
})
