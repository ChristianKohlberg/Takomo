import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { Canvas, type CanvasProps } from './Canvas'
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
    onTidy: vi.fn(),
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
      tidy: 'Tidy',
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
      trustLens: 'Trust',
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
    onTrustLens: vi.fn(),
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
