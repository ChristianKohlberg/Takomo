import * as Y from 'yjs'
import { createNode, readNodes } from '@/lib/mindmap-crdt'
import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { Canvas, type CanvasProps } from './Canvas'
import { layoutForMode } from '@/lib/mindmap-custom-layout'
import { NODE_HEIGHT, NODE_WIDTH, radialLayout } from '@/lib/mindmap-layout'

function props(over: Partial<CanvasProps> = {}): CanvasProps {
  return {
    title: 'Project specification',
    nodes: [],
    relationships: [],
    collapsed: new Set(),
    descendantCounts: new Map(),
    onToggleCollapse: vi.fn(),
    peers: [],
    selected: null,
    onSelect: vi.fn(),
    naming: null,
    onNameCommit: vi.fn(),
    onNameCancel: vi.fn(),
    onRenameNode: vi.fn(),
    onSibling: vi.fn(),
    onChild: vi.fn(),
    onAddBranch: vi.fn(),
    onDelete: vi.fn(),
    onReparent: vi.fn(),
    onPlace: vi.fn(),
    mode: 'radial',
    onMode: vi.fn(),
    relationFrom: null,
    onRelationTarget: vi.fn(),
    onCancelRelation: vi.fn(),
    canWrite: true,
    labels: {
      empty: 'Nothing here yet',
      emptyHint: 'Create a thought',
      fit: 'Fit',
      custom: 'Custom',
      radial: 'Radial',
      tree: 'Tree',
      zoomIn: 'Zoom in',
      zoomOut: 'Zoom out',
      expand: 'Expand',
      collapse: 'Collapse',
      cannotDrop: 'Cannot drop',
      pickRelationTarget: 'Pick a target',
      attachments: 'Attachments',
      addChild: 'Add child',
      nodeActions: 'Actions',
      nodeMenu: 'Menu',
      dropHere: 'Drop here',
      trustLegend: 'Trust legend',
      trustConfirmed: 'Confirmed',
      trustMachine: 'Machine',
      trustUnverified: 'Unverified',
      cutEdge: 'Detach',
      nameField: 'Title',
      nameHint: 'A few words',
    },
    cardLabels: {
      promoted: 'Promoted',
      hasNotes: 'Notes',
      hasRelations: 'Relations',
      originAgent: 'Agent',
      question: 'Question',
      folded: 'Folded',
      trustConfirmed: 'Confirmed',
      trustMachine: 'Machine',
      trustUnverified: 'Unverified',
      tests: 'Tests',
      testsFailing: 'Failing tests',
    },
    relationsFor: () => [],
    titleOf: new Map(),
    onOpenAttachments: vi.fn(),
    onAttachDrop: vi.fn(),
    pillVerbs: [],
    menuItemsFor: () => [],
    onRunVerb: vi.fn(),
    centreNode: null,
    onCentred: vi.fn(),
    fitRequest: null,
    onFitted: vi.fn(),
    focusRequest: null,
    onFocused: vi.fn(),
    foldSummaryOf: () => null,
    trustLens: false,
    onCreateAt: vi.fn(),
    onCutEdge: vi.fn(),
    ...over,
  }
}

describe('the specification map root', () => {
  it('stays visible without an empty-state overlay and adds a top-level section', () => {
    const p = props()
    render(<Canvas {...p} />)
    expect(screen.getByText(p.title)).toBeTruthy()
    expect(screen.queryByText(p.labels.empty)).toBeNull()
    expect(screen.queryByText(p.labels.emptyHint)).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Add child' }))
    expect(p.onAddBranch).toHaveBeenCalledOnce()
    expect(p.onChild).not.toHaveBeenCalled()
  })

  it('adds a top-level section when the root is double-clicked', () => {
    const p = props()
    render(<Canvas {...p} />)
    const { root } = radialLayout([])
    fireEvent.doubleClick(screen.getByRole('application'), {
      clientX: root.x + NODE_WIDTH / 2,
      clientY: root.y + NODE_HEIGHT / 2,
    })
    expect(p.onAddBranch).toHaveBeenCalledOnce()
    expect(p.onCreateAt).not.toHaveBeenCalled()
  })

  it('keeps read-only maps free of root creation actions', () => {
    const p = props({ canWrite: false })
    render(<Canvas {...p} />)
    expect(screen.queryByRole('button', { name: 'Add child' })).toBeNull()
    const { root } = radialLayout([])
    fireEvent.doubleClick(screen.getByRole('application'), {
      clientX: root.x + NODE_WIDTH / 2,
      clientY: root.y + NODE_HEIGHT / 2,
    })
    expect(p.onAddBranch).not.toHaveBeenCalled()
  })
})


it('refits layout changes unless the reader preserves the camera', () => {
  const rect = vi.spyOn(SVGSVGElement.prototype, 'getBoundingClientRect').mockReturnValue({ x: 0, y: 0, top: 0, left: 0, right: 900, bottom: 700, width: 900, height: 700, toJSON: () => ({}) })
  try {
    const doc = new Y.Doc()
    for (let i = 0; i < 20; i++) createNode(doc, { parent: null, title: `Branch ${i}`, by: 'test' })
    const p = props({ nodes: readNodes(doc) })
    const ui = render(<Canvas {...p} />)
    const camera = () => ui.container.querySelector('svg > g')?.getAttribute('transform')
    fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))
    const manual = camera()
    fireEvent.click(screen.getByRole('button', { name: 'Lock zoom and position' }))
    ui.rerender(<Canvas {...p} mode="tidy" />)
    expect(camera()).toBe(manual)
    fireEvent.click(screen.getByRole('button', { name: 'Lock zoom and position' }))
    ui.rerender(<Canvas {...p} mode="radial" />)
    expect(camera()).not.toBe(manual)
  } finally { rect.mockRestore() }
})


describe('layout views', () => {
  it('cycles Custom, Tree, and Radial using one button showing the current layout', () => {
    const p = props({ mode: 'custom' })
    const ui = render(<Canvas {...p} />)
    for (const [name, current, next] of [['Custom', 'custom', 'tidy'], ['Tree', 'tidy', 'radial'], ['Radial', 'radial', 'custom']] as const) {
      ui.rerender(<Canvas {...p} mode={current} />)
      const button = screen.getByRole('button', { name })
      expect(screen.getAllByRole('button', { name: /^(Custom|Tree|Radial)$/ })).toHaveLength(1)
      fireEvent.click(button)
      expect(p.onMode).toHaveBeenLastCalledWith(next)
    }
    expect(screen.queryByRole('button', { name: 'Tidy' })).toBeNull()
    expect(screen.queryByRole('button', { name: 'Trust' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Fit' }).textContent).toBe('')
  })

  it.each(['radial', 'tidy'] as const)('creates an unplaced branch on blank double-click in %s', mode => {
    const p = props({ mode })
    render(<Canvas {...p} />)
    fireEvent.doubleClick(screen.getByRole('application'), { clientX: 1500, clientY: 1200 })
    expect(p.onAddBranch).toHaveBeenCalledOnce()
    expect(p.onCreateAt).not.toHaveBeenCalled()
  })

  it('creates a positioned thought on blank double-click in Custom', () => {
    const p = props({ mode: 'custom' })
    render(<Canvas {...p} />)
    fireEvent.doubleClick(screen.getByRole('application'), { clientX: 1500, clientY: 1200 })
    expect(p.onCreateAt).toHaveBeenCalledWith({ x: 1500 - NODE_WIDTH / 2, y: 1200 - NODE_HEIGHT / 2 })
    expect(p.onAddBranch).not.toHaveBeenCalled()
  })
})

it('locks every camera movement and resumes navigation when unlocked', () => {
  const rect = vi.spyOn(SVGSVGElement.prototype, 'getBoundingClientRect').mockReturnValue({ x: 0, y: 0, top: 0, left: 0, right: 900, bottom: 700, width: 900, height: 700, toJSON: () => ({}) })
  try {
    const doc = new Y.Doc()
    for (let i = 0; i < 20; i++) createNode(doc, { parent: null, title: `Branch ${i}`, by: 'test' })
    const nodes = readNodes(doc)
    const p = props({ nodes, selected: nodes[0]!.id })
    const ui = render(<Canvas {...p} />)
    const camera = () => ui.container.querySelector('svg > g')?.getAttribute('transform')
    const canvas = screen.getByRole('application')
    fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))
    const lock = screen.getByRole('button', { name: 'Lock zoom and position' })
    fireEvent.click(lock)
    expect(lock.getAttribute('aria-pressed')).toBe('true')
    const frozen = camera()
    fireEvent.wheel(canvas, { deltaY: -100, clientX: 450, clientY: 350 })
    const pinch = new WheelEvent('wheel', { bubbles: true, cancelable: true, deltaY: 100, ctrlKey: true, clientX: 450, clientY: 350 })
    fireEvent(canvas, pinch)
    expect(pinch.defaultPrevented).toBe(true)
    for (const type of ['pointerdown', 'pointermove', 'pointerup']) {
      fireEvent(canvas, new MouseEvent(type, { bubbles: true, button: 0, clientX: type === 'pointerdown' ? -1000 : -800, clientY: -1000 }))
    }
    for (const name of ['Zoom in', 'Zoom out', 'Fit', 'Fit branch']) {
      const button = screen.getByRole('button', { name }) as HTMLButtonElement
      expect(button.disabled).toBe(true)
      fireEvent.click(button)
    }
    expect(camera()).toBe(frozen)
    ui.rerender(<Canvas {...p} mode="tidy" fitRequest={1} centreNode={nodes[10]!.id} />)
    expect(p.onFitted).toHaveBeenCalled()
    expect(p.onCentred).toHaveBeenCalled()
    expect(camera()).toBe(frozen)
    ui.rerender(<Canvas {...p} mode="tidy" fitRequest={null} centreNode={null} />)
    fireEvent.click(lock)
    expect(lock.getAttribute('aria-pressed')).toBe('false')
    expect(camera()).toBe(frozen)
    fireEvent.wheel(canvas, { deltaY: -100, clientX: 450, clientY: 350 })
    expect(camera()).not.toBe(frozen)
    fireEvent.click(screen.getByRole('button', { name: 'Fit' }))
    expect((screen.getByRole('button', { name: 'Fit' }) as HTMLButtonElement).disabled).toBe(false)
  } finally { rect.mockRestore() }
})

it('keeps the camera locked when the first synced nodes arrive', () => {
  const rect = vi.spyOn(SVGSVGElement.prototype, 'getBoundingClientRect').mockReturnValue({ x: 0, y: 0, top: 0, left: 0, right: 900, bottom: 700, width: 900, height: 700, toJSON: () => ({}) })
  try {
    const p = props()
    const ui = render(<Canvas {...p} />)
    fireEvent.click(screen.getByRole('button', { name: 'Lock zoom and position' }))
    const frozen = ui.container.querySelector('svg > g')?.getAttribute('transform')
    const doc = new Y.Doc()
    for (let i = 0; i < 20; i++) createNode(doc, { parent: null, title: `Branch ${i}`, by: 'test' })
    ui.rerender(<Canvas {...p} nodes={readNodes(doc)} />)
    expect(ui.container.querySelector('svg > g')?.getAttribute('transform')).toBe(frozen)
  } finally { rect.mockRestore() }
})


it.each(['custom', 'radial', 'tidy'] as const)('only permits node dragging in Custom, even with the camera locked (%s)', mode => {
  // jsdom lacks PointerEvent on some supported versions; a MouseEvent carries
  // the coordinate/button fields used by the canvas gesture handlers.
  const pointer = (target: Element, type: string, x: number, y: number) =>
    fireEvent(target, new MouseEvent(type, { bubbles: true, button: 0, clientX: x, clientY: y }))
  const doc = new Y.Doc()
  const id = createNode(doc, { parent: null, title: 'Planning', by: 'test' })!
  const nodes = readNodes(doc)
  const p = props({ nodes, mode })
  render(<Canvas {...p} />)
  const node = layoutForMode(nodes, mode).nodes.find(n => n.node.id === id)!
  fireEvent.click(screen.getByRole('button', { name: 'Lock zoom and position' }))
  const canvas = screen.getByRole('application')
  const x = node.x + NODE_WIDTH / 2
  const y = node.y + NODE_HEIGHT / 2
  pointer(canvas, 'pointerdown', x, y)
  pointer(canvas, 'pointermove', x + 900, y + 800)
  pointer(canvas, 'pointerup', x + 900, y + 800)
  expect(p.onSelect).toHaveBeenCalledWith(id)
  if (mode === 'custom') expect(p.onPlace).toHaveBeenCalledWith(id, { x: node.x + 900, y: node.y + 800 })
  else expect(p.onPlace).not.toHaveBeenCalled()
  expect(p.onReparent).not.toHaveBeenCalled()
})

it('highlights matches without selecting nodes or moving the camera', () => {
  const doc = new Y.Doc()
  const id = createNode(doc, { parent: null, title: 'Delivery', by: 'test' })!
  const p = props({ nodes: readNodes(doc) })
  const ui = render(<Canvas {...p} />)
  const camera = () => ui.container.querySelector('svg > g')?.getAttribute('transform')
  const before = camera()
  ui.rerender(<Canvas {...p} searchMatches={new Set([id])} trustLens />)
  expect(ui.container.querySelectorAll('[data-search-match="true"]')).toHaveLength(1)
  expect(camera()).toBe(before)
  expect(p.onSelect).not.toHaveBeenCalled()
  ui.rerender(<Canvas {...p} searchMatches={new Set()} />)
  expect(ui.container.querySelector('[data-search-match]')).toBeNull()
})

it.each(['custom', 'radial', 'tidy'] as const)('fits %s focus once and restores the full-map camera after nested focus', mode => {
  const rect = vi.spyOn(SVGSVGElement.prototype, 'getBoundingClientRect').mockReturnValue({ x: 0, y: 0, top: 0, left: 0, right: 900, bottom: 700, width: 900, height: 700, toJSON: () => ({}) })
  try {
    const doc = new Y.Doc()
    const root = createNode(doc, { parent: null, title: 'Payments', by: 'test' })!
    const child = createNode(doc, { parent: root, title: 'Retry', by: 'test' })!
    const nodes = readNodes(doc), p = props({ nodes, mode })
    const ui = render(<Canvas {...p} />)
    const camera = () => ui.container.querySelector('svg > g')?.getAttribute('transform')
    fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))
    const before = camera()
    ui.rerender(<Canvas {...p} focusRoot={root} />)
    expect(screen.queryByText(p.title)).toBeNull()
    expect(camera()).not.toBe(before)
    fireEvent.click(screen.getByRole('button', { name: 'Zoom in' }))
    const zoomed = camera()
    ui.rerender(<Canvas {...p} focusRoot={root} searchMatches={new Set([child])} />)
    expect(camera()).toBe(zoomed)
    ui.rerender(<Canvas {...p} focusRoot={child} nodes={[{ ...nodes[1]!, parent: null }]} />)
    ui.rerender(<Canvas {...p} focusRoot={null} />)
    expect(camera()).toBe(before)
    expect(p.onPlace).not.toHaveBeenCalled()
    expect(p.onReparent).not.toHaveBeenCalled()
  } finally { rect.mockRestore() }
})

it('respects the camera lock while entering and leaving focus', () => {
  const rect = vi.spyOn(SVGSVGElement.prototype, 'getBoundingClientRect').mockReturnValue({ x: 0, y: 0, top: 0, left: 0, right: 900, bottom: 700, width: 900, height: 700, toJSON: () => ({}) })
  try {
    const doc = new Y.Doc()
    const id = createNode(doc, { parent: null, title: 'Payments', by: 'test' })!
    const p = props({ nodes: readNodes(doc) })
    const ui = render(<Canvas {...p} />)
    fireEvent.click(screen.getByRole('button', { name: 'Lock zoom and position' }))
    const before = ui.container.querySelector('svg > g')?.getAttribute('transform')
    ui.rerender(<Canvas {...p} focusRoot={id} />)
    expect(ui.container.querySelector('svg > g')?.getAttribute('transform')).toBe(before)
    ui.rerender(<Canvas {...p} focusRoot={null} />)
    expect(ui.container.querySelector('svg > g')?.getAttribute('transform')).toBe(before)
  } finally { rect.mockRestore() }
})
