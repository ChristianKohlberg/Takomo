// The palette's keyboard, which is the part of it that is not a list.
//
// jsdom has no layout engine, so nothing here says the overlay LOOKS right. What
// it does say is that the row a keystroke runs is the row that was highlighted —
// the failure that matters, because a palette that runs the wrong command is
// worse than one that runs none. Which commands exist is a separate, pure
// question, tested in `lib/mindmap-commands.test.ts`.
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'

import { CommandPalette, type PaletteItem } from './CommandPalette'

const LABELS = {
  scopeNode: 'This thought',
  scopeMap: 'This map',
  placeholder: 'What do you want to do?',
  noMatch: 'Nothing matches',
  keys: '↑↓ move · Enter run · Esc close',
}

const ITEMS: PaletteItem[] = [
  { id: 'a', label: 'Add a thought underneath' },
  { id: 'b', label: 'Rename this thought' },
  { id: 'c', label: 'Remove this branch', hint: 'Everything under it goes too.' },
]

function Harness({
  items = ITEMS,
  onRun = () => {},
  onClose = () => {},
}: {
  items?: PaletteItem[]
  onRun?: (id: string) => void
  onClose?: () => void
}) {
  const [active, setActive] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  return (
    <CommandPalette
      scope="A thought"
      scopeKind="node"
      items={items}
      query={query}
      onQuery={setQuery}
      active={active}
      onActive={setActive}
      onRun={onRun}
      onClose={onClose}
      labels={LABELS}
    />
  )
}

const input = () => screen.getByLabelText(LABELS.placeholder)

describe('CommandPalette', () => {
  it('says what it is scoped to', () => {
    render(<Harness />)
    expect(screen.getByRole('dialog').textContent).toContain('This thought')
    expect(screen.getByRole('dialog').textContent).toContain('A thought')
  })

  it('runs the first row when nothing has been arrowed to', () => {
    const onRun = vi.fn()
    render(<Harness onRun={onRun} />)
    fireEvent.keyDown(input(), { key: 'Enter' })
    expect(onRun).toHaveBeenCalledWith('a')
  })

  it('moves the highlight with the arrows and runs what is highlighted', () => {
    const onRun = vi.fn()
    render(<Harness onRun={onRun} />)
    fireEvent.keyDown(input(), { key: 'ArrowDown' })
    fireEvent.keyDown(input(), { key: 'ArrowDown' })
    fireEvent.keyDown(input(), { key: 'Enter' })
    expect(onRun).toHaveBeenCalledWith('c')
  })

  it('wraps around rather than sticking at the ends', () => {
    const onRun = vi.fn()
    render(<Harness onRun={onRun} />)
    fireEvent.keyDown(input(), { key: 'ArrowUp' })
    fireEvent.keyDown(input(), { key: 'Enter' })
    expect(onRun).toHaveBeenCalledWith('c')
  })

  it('closes on Escape and on ⌘K, which the page shortcut cannot do from here', () => {
    const onClose = vi.fn()
    render(<Harness onClose={onClose} />)
    fireEvent.keyDown(input(), { key: 'Escape' })
    fireEvent.keyDown(input(), { key: 'k', metaKey: true })
    expect(onClose).toHaveBeenCalledTimes(2)
  })

  it('says so when nothing matches instead of showing an empty box', () => {
    render(<Harness items={[]} />)
    expect(screen.getByText(LABELS.noMatch)).toBeTruthy()
    // Enter on an empty list must not run anything.
    fireEvent.keyDown(input(), { key: 'Enter' })
  })

  it('runs a row that is clicked', () => {
    const onRun = vi.fn()
    render(<Harness onRun={onRun} />)
    fireEvent.click(screen.getByText('Rename this thought'))
    expect(onRun).toHaveBeenCalledWith('b')
  })
})
