// A card is a title, its marks, and one line of substance — the same card
// whether or not it is selected, because selecting a node is not opening it.
// The detail it used to grow into is `NodeDialog`, and those cases moved there
// with it.
//
// The one thing you type here is a TITLE, so the "no inputs" invariant is now
// two rules rather than one: exactly one input while the node is being named,
// and none at any other time.
import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'

import { NodeCard, type NodeCardLabels } from './NodeCard'
import type { MapNode, Relationship } from '@/lib/mindmap-doc'

const LABELS: NodeCardLabels = {
  promoted: 'became',
  hasNotes: 'Has notes',
  hasRelations: 'Has relations',
  originAgent: 'An agent wrote this',
  question: 'open question',
  folded: 'Folded — {n} thoughts under this one',
  trustConfirmed: 'A person wrote this and confirmed it',
  trustMachine: 'An agent wrote it and nobody has checked it',
  tests: '{n} tests',
  testsFailing: '{n} tests, {m} not passing',
  trustUnverified: 'A thought nobody has confirmed yet',
}

const NAME_LABELS = { field: 'Title of this thought', hint: 'A few words' }

const node = (over: Partial<MapNode> = {}): MapNode => ({
  id: 'mn-1',
  parent: null,
  order: 'a0',
  title: 'Pricing',
  notes: 'the long form, in full',
  at: null,
  edge_label: '',
  kind: 'thought',
  origin: 'agent',
  reviewed: false,
  icons: [],
  color: '',
  shape: 'rounded',
  attachments: [{ id: 'ma-1', kind: 'pdf', name: 'spec.pdf', gist: 'the rules', ref: '' }],
  promoted: { kind: 'epic', id: 'tk-9' },
  created_by: 'ada',
  created_at: 0,
  updated_at: 0,
  position: 0,
  ...over,
})

const relations: Relationship[] = [{ id: 'mr-1', from: 'mn-1', to: 'mn-2', label: 'blocks' }]

function mount(over: Partial<Parameters<typeof NodeCard>[0]> = {}) {
  return render(
    <NodeCard node={node()} relations={relations} fold={null} trust={null} labels={LABELS} {...over} />,
  )
}

describe('a card on the map', () => {
  it('says where the substance is without drawing any of it', () => {
    // The marks are the always-on signal that there is something to open. The
    // notes themselves, the attachment list and the relations are in the dialog,
    // because a click on the map must not throw a panel across it.
    mount()
    expect(screen.getByText('Pricing')).toBeTruthy()
    expect(screen.getByTitle('Has notes')).toBeTruthy()
    expect(screen.getByTitle('Has relations')).toBeTruthy()
    expect(screen.getByTitle('An agent wrote this')).toBeTruthy()
    expect(screen.getByTitle('became tk-9')).toBeTruthy()
    // One quiet line of what it says — never the whole paragraph.
    expect(screen.getByText('the long form, in full')).toBeTruthy()
    expect(screen.queryByText(/spec\.pdf/)).toBeNull()
  })

  it('has nothing on it to type into or press', () => {
    const { container } = mount()
    expect(container.querySelectorAll('input, textarea, select, button')).toHaveLength(0)
  })

  it('is the same card when it is the selected one', () => {
    // There is no `expanded` any more: selection is a highlight the canvas draws
    // around this, and the card itself does not know about it.
    const { container } = mount()
    expect(container.querySelectorAll('input, textarea, select, button')).toHaveLength(0)
    expect(screen.queryByRole('textbox')).toBeNull()
  })

  it('says what a folded branch is holding rather than hiding it', () => {
    mount({ fold: { count: 3, text: 'Tiers · Discounts · Trials' } })
    expect(screen.getByText('Tiers · Discounts · Trials')).toBeTruthy()
    expect(screen.getByTitle('Folded — 3 thoughts under this one')).toBeTruthy()
  })

  it('lets a keystroke, a press and the wheel through to the canvas', () => {
    // The card catches nothing at all now: it has no scrolling detail to protect
    // from the zoom, and the map keyboard — ⌘K, the arrows, Delete — lives on the
    // canvas underneath it.
    const onKeyDown = vi.fn()
    const onPointerDown = vi.fn()
    const onWheel = vi.fn()
    const { container } = render(
      <div onKeyDown={onKeyDown} onPointerDown={onPointerDown} onWheel={onWheel}>
        <NodeCard node={node()} relations={relations} fold={null} trust={null} labels={LABELS} />
      </div>,
    )
    const card = container.firstElementChild?.firstElementChild as HTMLElement
    fireEvent.keyDown(card, { key: 'k' })
    fireEvent.pointerDown(card)
    fireEvent.wheel(card)
    expect(onKeyDown).toHaveBeenCalled()
    expect(onPointerDown).toHaveBeenCalled()
    expect(onWheel).toHaveBeenCalled()
  })
})

describe('a card being named', () => {
  const naming = (over: Partial<Parameters<typeof NodeCard>[0]> = {}) => {
    const onCommit = vi.fn()
    const onCancel = vi.fn()
    const rendered = mount({
      naming: { onCommit, onCancel, labels: NAME_LABELS },
      ...over,
    })
    return { onCommit, onCancel, ...rendered }
  }

  it('is the title line and a caret, and nothing else', () => {
    // Not the marks, not the substance: the thing being named should be the
    // thing being looked at.
    const { container } = naming()
    const input = screen.getByLabelText('Title of this thought') as HTMLInputElement
    expect(input.value).toBe('Pricing')
    expect(document.activeElement).toBe(input)
    // The placeholder is meant to be typed over, so it arrives selected.
    expect(input.selectionStart).toBe(0)
    expect(input.selectionEnd).toBe('Pricing'.length)
    expect(container.querySelectorAll('input, textarea, select, button')).toHaveLength(1)
    expect(screen.queryByTitle('Has notes')).toBeNull()
    expect(screen.queryByText('the long form, in full')).toBeNull()
  })

  it('commits on Enter and goes a level deeper on Tab', () => {
    const { onCommit } = naming()
    const input = screen.getByLabelText('Title of this thought')
    fireEvent.change(input, { target: { value: 'Pricing v2' } })
    expect(onCommit).not.toHaveBeenCalled()
    fireEvent.keyDown(input, { key: 'Enter' })
    // 'stay' is what keeps the node selected, so the next Enter on the canvas
    // makes its next sibling and the fast loop is a loop.
    expect(onCommit).toHaveBeenCalledWith('Pricing v2', 'stay')
  })

  it('goes a level deeper on Tab', () => {
    const { onCommit } = naming()
    const input = screen.getByLabelText('Title of this thought')
    fireEvent.change(input, { target: { value: 'Tiers' } })
    fireEvent.keyDown(input, { key: 'Tab' })
    expect(onCommit).toHaveBeenCalledWith('Tiers', 'child')
  })

  it('abandons on Escape without committing anything', () => {
    const { onCommit, onCancel } = naming()
    const input = screen.getByLabelText('Title of this thought')
    fireEvent.change(input, { target: { value: 'half a thou' } })
    fireEvent.keyDown(input, { key: 'Escape' })
    expect(onCancel).toHaveBeenCalled()
    expect(onCommit).not.toHaveBeenCalled()
    // …and a blur on the way out does not then commit what was abandoned.
    fireEvent.blur(input)
    expect(onCommit).not.toHaveBeenCalled()
  })

  it('commits once when the caret loses the focus, and not twice', () => {
    const { onCommit } = naming()
    const input = screen.getByLabelText('Title of this thought')
    fireEvent.keyDown(input, { key: 'Enter' })
    fireEvent.blur(input)
    expect(onCommit).toHaveBeenCalledTimes(1)
  })

  it('keeps every keystroke off the canvas', () => {
    // Delete would prune the branch being named, and a press would pan the map.
    const onKeyDown = vi.fn()
    const onPointerDown = vi.fn()
    render(
      <div onKeyDown={onKeyDown} onPointerDown={onPointerDown}>
        <NodeCard
          node={node()}
          relations={relations}
          fold={null}
          trust={null}
          naming={{ onCommit: vi.fn(), onCancel: vi.fn(), labels: NAME_LABELS }}
          labels={LABELS}
        />
      </div>,
    )
    const input = screen.getByLabelText('Title of this thought')
    fireEvent.keyDown(input, { key: 'Delete' })
    fireEvent.pointerDown(input)
    expect(onKeyDown).not.toHaveBeenCalled()
    expect(onPointerDown).not.toHaveBeenCalled()
  })

  it('gives a read-only card no caret at all', () => {
    // `naming` is null on every card a token cannot write to — the canvas checks
    // it again for the same reason every entrance does.
    const { container } = mount({ naming: null })
    expect(container.querySelectorAll('input')).toHaveLength(0)
  })

  it('says how many tests a section carries, and calls out the failing ones', () => {
    // A count and never a verdict: the map says where the verification is, and
    // /verification is where you read what it says.
    const { unmount } = mount({ tests: { total: 3, failing: 0 } })
    expect(screen.getByTitle('3 tests')).toBeTruthy()
    unmount()

    mount({ tests: { total: 3, failing: 1 } })
    expect(screen.getByTitle('3 tests, 1 not passing')).toBeTruthy()
  })

  it('draws no test mark for a section with none', () => {
    mount({ tests: null })
    expect(screen.queryByTitle(/tests/)).toBeNull()
  })
})
