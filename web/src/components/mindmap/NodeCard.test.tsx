// The card is text you read. That is the whole rule this file exists to keep:
// there is no control on it, and it no longer eats the events the canvas needs —
// which is what let the map keyboard and dragging the selected node work at all
// while the card was also a form.
import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/react'

import { NodeCard, type NodeCardLabels } from './NodeCard'
import type { MapNode, Relationship } from '@/lib/mindmap-doc'

const LABELS: NodeCardLabels = {
  notes: 'Notes',
  attachments: 'Attachments ({n})',
  relations: 'Relations',
  noRelations: 'No relations from this thought.',
  promoted: 'became',
  origin: 'Written by',
  originHuman: 'person',
  originAgent: 'agent',
  hasNotes: 'Has notes',
  hasRelations: 'Has relations',
  question: 'open question',
  folded: 'Folded — {n} thoughts under this one',
  trustConfirmed: 'A person wrote this and confirmed it',
  trustMachine: 'An agent wrote it and nobody has checked it',
  trustUnverified: 'A thought nobody has confirmed yet',
}

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
    <NodeCard
      node={node()}
      expanded
      relations={relations}
      titleOf={new Map([['mn-2', 'Billing']])}
      fold={null}
      trust={null}
      labels={LABELS}
      {...over}
    />,
  )
}

describe('the expanded card', () => {
  it('reads out everything the node holds', () => {
    mount()
    expect(screen.getByText('Pricing')).toBeTruthy()
    expect(screen.getByText('the long form, in full')).toBeTruthy()
    expect(screen.getByText('Attachments (1)')).toBeTruthy()
    expect(screen.getByText(/spec\.pdf/)).toBeTruthy()
    expect(screen.getByText(/Billing/)).toBeTruthy()
    expect(screen.getByText(/became tk-9/)).toBeTruthy()
    expect(screen.getByText(/Written by agent/)).toBeTruthy()
  })

  it('has nothing on it to type into or press', () => {
    // The one invariant. Every control that used to be here is in NodeDialog,
    // because a form on the canvas has to fight the canvas for the keyboard.
    const { container } = mount()
    expect(container.querySelectorAll('input, textarea, select, button')).toHaveLength(0)
  })

  it('lets a keystroke and a press through to the canvas, and keeps the wheel', () => {
    // Keys reach the map: ⌘K, the arrows and Delete all live on the canvas. So
    // does the press that drags the selected node, which used to be possible
    // only from a grip. The wheel is the exception — scrolling long notes must
    // not zoom the map behind them.
    const onKeyDown = vi.fn()
    const onPointerDown = vi.fn()
    const onWheel = vi.fn()
    const { container } = render(
      <div onKeyDown={onKeyDown} onPointerDown={onPointerDown} onWheel={onWheel}>
        <NodeCard
          node={node()}
          expanded
          relations={relations}
          titleOf={new Map()}
          fold={null}
          trust={null}
          labels={LABELS}
        />
      </div>,
    )
    const card = container.firstElementChild?.firstElementChild as HTMLElement
    fireEvent.keyDown(card, { key: 'k' })
    fireEvent.pointerDown(card)
    fireEvent.wheel(card)
    expect(onKeyDown).toHaveBeenCalled()
    expect(onPointerDown).toHaveBeenCalled()
    expect(onWheel).not.toHaveBeenCalled()
  })
})

describe('an unselected card', () => {
  it('stays a title, its marks and one line of substance', () => {
    const { container } = mount({ expanded: false })
    expect(screen.getByText('Pricing')).toBeTruthy()
    expect(screen.getByTitle('Has notes')).toBeTruthy()
    expect(screen.getByTitle('Has relations')).toBeTruthy()
    // The notes are a mark and a first sentence here, never the whole paragraph.
    expect(screen.queryByText('Notes')).toBeNull()
    expect(container.querySelectorAll('input, textarea, select, button')).toHaveLength(0)
  })

  it('says what a folded branch is holding rather than hiding it', () => {
    mount({
      expanded: false,
      fold: { count: 3, text: 'Tiers · Discounts · Trials' },
    })
    expect(screen.getByText('Tiers · Discounts · Trials')).toBeTruthy()
    expect(screen.getByTitle('Folded — 3 thoughts under this one')).toBeTruthy()
  })
})
