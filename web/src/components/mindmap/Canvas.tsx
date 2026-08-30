// The mindmap canvas: pan, zoom, drag, relations, and a keyboard that keeps up
// with talking — now with other people drawing on it at the same time.
//
// Every decision here is made by a pure function in `lib/mindmap-layout.ts` —
// where a node lands, which node a point is on, what a drop means, how zoom
// tracks the cursor. This file draws the result and turns pointer and key events
// into calls. That split is the only way any of it is testable: jsdom has no
// layout engine, so a component test can prove nothing about geometry.
//
// Hand-rolled rather than a graph library, deliberately. `@xyflow/react` and its
// peers are 50–100 kB gzipped against ~70 kB of vendor-budget headroom, and what
// a mindmap needs from one — pan, zoom, drag, hit-test, tidy-tree — is the part
// that fits in a few hundred lines. This repo hand-rolls its markdown renderer,
// its typeahead and its base64url for the same reason.
//
// It takes a tree, not a document: the CRDT lives one level up, so this stays a
// renderer with no idea that the nodes it is drawing arrived over a socket.
//
// The side panel is gone and its job moved ONTO the node: the selected card
// grows into `NodeCard`'s full form, every other one stays a title and its marks.
// That is why the card's size is not a layout decision — the geometry in
// `lib/mindmap-layout.ts` still places every node by `NODE_WIDTH`/`NODE_HEIGHT`,
// and the expanded card is drawn over its neighbours rather than pushing them.
// Re-laying out the map around whatever is selected would move every other node
// under a collaborator's cursor on every click.
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import {
  DEFAULT_VIEWPORT,
  centreOn,
  MAX_ZOOM,
  MIN_ZOOM,
  NODE_HEIGHT,
  NODE_WIDTH,
  edgePath,
  fit,
  layout,
  nodeAt,
  radialLayout,
  resolveDrop,
  toWorld,
  zoomAt,
  type Point,
  type Viewport,
} from '@/lib/mindmap-layout'
import { cn } from '@/lib/utils'
import type { AttachmentKind, MapNode, NodeFields, Relationship } from '@/lib/mindmap-doc'
import { Hint } from '@/components/Hint'
import {
  EXPANDED_HEIGHT,
  EXPANDED_WIDTH,
  NodeCard,
  type NodeCardLabels,
  type Reveal,
} from '@/components/mindmap/NodeCard'

/** How the first ring is arranged. Per-viewer, like pan and zoom. */
export type CanvasMode = 'radial' | 'tidy'

/** Somebody else in the same map, and the node they have selected. */
export interface CanvasPeer {
  name: string
  color: string
  selected: string | null
}

export interface CanvasLabels {
  /** The root box's caption, when the map has no nodes yet. */
  empty: string
  emptyHint: string
  fit: string
  tidy: string
  radial: string
  tree: string
  zoomIn: string
  zoomOut: string
  expand: string
  collapse: string
  /** Announced on the drag target while a drop would be refused. */
  cannotDrop: string
  /** Shown while a relation is waiting for its second node. */
  pickRelationTarget: string
}

export interface CanvasProps {
  title: string
  /** Already filtered for this viewer's folds. */
  nodes: MapNode[]
  relationships: Relationship[]
  /** Branches this viewer has folded, and how many thoughts sit under each node
   *  in the WHOLE tree — the fold handle needs both, and only one of them
   *  survives the filter that produced `nodes`. */
  collapsed: ReadonlySet<string>
  descendantCounts: ReadonlyMap<string, number>
  onToggleCollapse: (id: string) => void
  peers: CanvasPeer[]
  selected: string | null
  onSelect: (id: string | null) => void
  /** Commit an edited node's title. Called on blur or Escape, not per keystroke. */
  onTitle: (id: string, title: string) => void
  /** Enter: a sibling after this node. Tab: a child of it. */
  onSibling: (id: string) => void
  onChild: (id: string) => void
  onDelete: (id: string) => void
  onReparent: (id: string, parent: string) => void
  onPlace: (id: string, at: Point) => void
  /** Clear every hand placement and let the layout take over again. */
  onTidy: () => void
  mode: CanvasMode
  onMode: (mode: CanvasMode) => void
  /** The node a relation is being drawn FROM, or null. */
  relationFrom: string | null
  /** The second click of that gesture. */
  onRelationTarget: (id: string) => void
  /** Escape, or a click into empty space, abandons it. */
  onCancelRelation: () => void
  canWrite: boolean
  labels: CanvasLabels
  /** Everything the selected card needs to be the detail surface. */
  cardLabels: NodeCardLabels
  relationsFor: (id: string) => Relationship[]
  titleOf: ReadonlyMap<string, string>
  onNotes: (id: string, notes: string) => void
  onFields: (id: string, fields: Partial<NodeFields>) => void
  onAddAttachment: (
    id: string,
    draft: { kind: AttachmentKind; name: string; gist: string; ref: string },
  ) => void
  onRemoveAttachment: (id: string, attachmentId: string) => void
  onRemoveRelation: (relationId: string) => void
  /**
   * Asks from ⌘K, each cleared the moment it is honoured: bring a node into the
   * middle of the view, open its title editor, or reveal one part of its card.
   */
  centreNode: string | null
  onCentred: () => void
  /** A timestamp, so asking twice asks twice. Null while nothing is pending. */
  fitRequest: number | null
  onFitted: () => void
  editNode: string | null
  onEditOpened: () => void
  reveal: Reveal
  onRevealed: () => void
  className?: string
}

/** How far a pointer must move before a press counts as a drag, not a click. */
const DRAG_THRESHOLD = 4

type Drag =
  | { kind: 'none' }
  | { kind: 'pan'; from: Point; viewport: Viewport }
  | {
      kind: 'node'
      id: string
      /** Where in the node the pointer grabbed it, world units. */
      grab: Point
      /** Live top-left while dragging, world units. */
      at: Point
      moved: boolean
    }

/** The one place `shape` becomes geometry. A square node reads as a screen, a
 *  pill as a label; anything unrecognised falls back to the ordinary box. */
function cornerRadius(shape: string): number {
  if (shape === 'square') return 2
  if (shape === 'pill') return NODE_HEIGHT / 2
  return 9
}

export function Canvas({
  title,
  nodes,
  relationships,
  collapsed,
  descendantCounts,
  onToggleCollapse,
  peers,
  selected,
  onSelect,
  onTitle,
  onSibling,
  onChild,
  onDelete,
  onReparent,
  onPlace,
  onTidy,
  mode,
  onMode,
  relationFrom,
  onRelationTarget,
  onCancelRelation,
  canWrite,
  labels,
  cardLabels,
  relationsFor,
  titleOf,
  onNotes,
  onFields,
  onAddAttachment,
  onRemoveAttachment,
  onRemoveRelation,
  centreNode,
  onCentred,
  fitRequest,
  onFitted,
  editNode,
  onEditOpened,
  reveal,
  onRevealed,
  className,
}: CanvasProps) {
  const svgRef = useRef<SVGSVGElement | null>(null)
  const [viewport, setViewport] = useState<Viewport>(DEFAULT_VIEWPORT)
  const [drag, setDrag] = useState<Drag>({ kind: 'none' })
  const [editing, setEditing] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const fitted = useRef(false)

  const placed = mode === 'radial' ? radialLayout(nodes) : layout(nodes)
  const byId = new Map(placed.nodes.map((p) => [p.node.id, p]))
  // The selected card is drawn LAST, because it is the only one that grows: in
  // SVG there is no z-index, so paint order is the only way it lands over its
  // neighbours rather than under them.
  const ordered = [...placed.nodes].sort(
    (a, b) => Number(a.node.id === selected) - Number(b.node.id === selected),
  )

  // Fit once, when the map first arrives with something in it. Refitting on every
  // change would yank the view out from under somebody who had panned somewhere —
  // and on a shared canvas that somebody might be a collaborator typing.
  useLayoutEffect(() => {
    if (fitted.current || nodes.length === 0) return
    const box = svgRef.current?.getBoundingClientRect()
    if (!box || box.width === 0) return
    setViewport(fit(placed.bounds, box.width, box.height))
    fitted.current = true
  }, [nodes.length, placed.bounds])

  const pointIn = useCallback((e: { clientX: number; clientY: number }): Point => {
    const box = svgRef.current?.getBoundingClientRect()
    return { x: e.clientX - (box?.left ?? 0), y: e.clientY - (box?.top ?? 0) }
  }, [])

  /**
   * The node under a point — including the SELECTED one's expanded card.
   *
   * `nodeAt` knows only the layout's box, and the expanded card is wider than
   * it. The card itself stops every pointer event except on its grip, so what
   * reaches here from that region is exactly the press that means "drag me" —
   * and without this it would land in empty space and deselect instead.
   */
  const hitAt = (world: Point) => {
    const direct = nodeAt(placed.nodes, world)
    if (direct) return direct
    const p = selected ? byId.get(selected) : undefined
    if (!p) return null
    const at = positionOf(p.node.id)
    const left = at.x + (NODE_WIDTH - EXPANDED_WIDTH) / 2
    const inside =
      world.x >= left &&
      world.x <= left + EXPANDED_WIDTH &&
      world.y >= at.y &&
      world.y <= at.y + EXPANDED_HEIGHT
    return inside ? p : null
  }

  const onPointerDown = (e: React.PointerEvent<SVGSVGElement>) => {
    if (editing) return
    const screen = pointIn(e)
    const world = toWorld(screen, viewport)
    const hit = hitAt(world)
    // Drawing a relation is a two-click gesture, so a press while it is armed is
    // the second click and nothing else — never the start of a drag.
    if (relationFrom) {
      // A click into empty space abandons the gesture rather than doing nothing:
      // a mode you cannot see your way out of is worse than no mode.
      if (hit) onRelationTarget(hit.node.id)
      else onCancelRelation()
      return
    }
    ;(e.target as Element).setPointerCapture?.(e.pointerId)
    if (hit) {
      onSelect(hit.node.id)
      setDrag({
        kind: 'node',
        id: hit.node.id,
        grab: { x: world.x - hit.x, y: world.y - hit.y },
        at: { x: hit.x, y: hit.y },
        moved: false,
      })
    } else {
      onSelect(null)
      setDrag({ kind: 'pan', from: screen, viewport })
    }
  }

  const onPointerMove = (e: React.PointerEvent<SVGSVGElement>) => {
    if (drag.kind === 'none') return
    const screen = pointIn(e)
    if (drag.kind === 'pan') {
      setViewport({
        ...drag.viewport,
        x: drag.viewport.x + (screen.x - drag.from.x),
        y: drag.viewport.y + (screen.y - drag.from.y),
      })
      return
    }
    const world = toWorld(screen, viewport)
    const at = { x: world.x - drag.grab.x, y: world.y - drag.grab.y }
    const moved =
      drag.moved ||
      Math.abs(at.x - drag.at.x) * viewport.zoom > DRAG_THRESHOLD ||
      Math.abs(at.y - drag.at.y) * viewport.zoom > DRAG_THRESHOLD
    setDrag({ ...drag, at, moved })
  }

  const onPointerUp = (e: React.PointerEvent<SVGSVGElement>) => {
    if (drag.kind === 'node' && drag.moved && canWrite) {
      const node = nodes.find((n) => n.id === drag.id)
      if (node) {
        // The pointer decides the drop, not the node's own box: dropping is aimed
        // with the cursor.
        const world = toWorld(pointIn(e), viewport)
        const drop = resolveDrop(placed.nodes, node, world, drag.at)
        if (drop.kind === 'reparent') onReparent(drag.id, drop.parent)
        else if (drop.kind === 'place') onPlace(drag.id, drop.at)
      }
    }
    setDrag({ kind: 'none' })
  }

  const onWheel = (e: React.WheelEvent<SVGSVGElement>) => {
    // A trackpad pinch arrives as a wheel event with ctrlKey; both mean zoom here,
    // and neither should scroll the page behind the canvas.
    e.preventDefault()
    setViewport((v) => zoomAt(v, pointIn(e), e.deltaY < 0 ? 1.1 : 1 / 1.1))
  }

  const startEditing = useCallback(
    (id: string) => {
      const node = nodes.find((n) => n.id === id)
      if (!node || !canWrite) return
      setDraft(node.title)
      setEditing(id)
    },
    [nodes, canWrite],
  )

  const commit = useCallback(() => {
    if (editing) onTitle(editing, draft)
    setEditing(null)
  }, [editing, draft, onTitle])

  // ⌘K asked for a node. Centring is deliberately not a fit: it keeps the zoom
  // the reader chose, because jumping AND rezooming loses them twice. The layout
  // is recomputed here rather than read from the render pass so this effect can
  // depend on the nodes honestly — it does nothing at all unless an ask is
  // pending, which the caller clears the moment it is honoured.
  useEffect(() => {
    if (!centreNode) return
    const box = svgRef.current?.getBoundingClientRect()
    const at = (mode === 'radial' ? radialLayout(nodes) : layout(nodes)).nodes.find(
      (p) => p.node.id === centreNode,
    )
    if (box && at) {
      setViewport((v) =>
        centreOn(
          v,
          { x: at.x + NODE_WIDTH / 2, y: at.y + NODE_HEIGHT / 2 },
          box.width,
          box.height,
        ),
      )
    }
    onCentred()
  }, [centreNode, onCentred, nodes, mode])

  useEffect(() => {
    if (fitRequest === null) return
    const box = svgRef.current?.getBoundingClientRect()
    const bounds = (mode === 'radial' ? radialLayout(nodes) : layout(nodes)).bounds
    if (box) setViewport(fit(bounds, box.width, box.height))
    onFitted()
  }, [fitRequest, onFitted, nodes, mode])

  useEffect(() => {
    if (!editNode) return
    startEditing(editNode)
    onEditOpened()
  }, [editNode, onEditOpened, startEditing])

  // The keyboard is what makes this keep up with a conversation, so it lives on
  // the canvas rather than only inside a text box: with a node selected, Enter
  // and Tab grow the map without the mouse.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (editing) return
    if (e.key === 'Escape' && relationFrom) {
      e.preventDefault()
      onCancelRelation()
      return
    }
    if (!selected) return
    if (e.key === 'Enter') {
      e.preventDefault()
      onSibling(selected)
    } else if (e.key === 'Tab') {
      e.preventDefault()
      onChild(selected)
    } else if (e.key === 'F2') {
      e.preventDefault()
      startEditing(selected)
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault()
      onDelete(selected)
    } else if (e.key === ' ') {
      e.preventDefault()
      onToggleCollapse(selected)
    } else if (e.key === 'Escape') {
      onSelect(null)
    }
  }

  const zoomBy = (factor: number) => {
    const box = svgRef.current?.getBoundingClientRect()
    setViewport((v) => zoomAt(v, { x: (box?.width ?? 0) / 2, y: (box?.height ?? 0) / 2 }, factor))
  }

  const fitAll = () => {
    const box = svgRef.current?.getBoundingClientRect()
    if (box) setViewport(fit(placed.bounds, box.width, box.height))
  }

  /** Where a node is drawn right now — its dragged position while dragging. */
  const positionOf = (id: string): Point => {
    if (drag.kind === 'node' && drag.id === id && drag.moved) return drag.at
    const p = byId.get(id)
    return p ? { x: p.x, y: p.y } : { x: 0, y: 0 }
  }

  const centreOf = (id: string): Point => {
    const at = positionOf(id)
    return { x: at.x + NODE_WIDTH / 2, y: at.y + NODE_HEIGHT / 2 }
  }

  const dropTarget =
    drag.kind === 'node' && drag.moved
      ? (() => {
          const node = nodes.find((n) => n.id === drag.id)
          if (!node) return null
          const pointer = { x: drag.at.x + drag.grab.x, y: drag.at.y + drag.grab.y }
          const over = nodeAt(placed.nodes, pointer)
          if (!over || over.node.id === drag.id) return null
          const drop = resolveDrop(placed.nodes, node, pointer, drag.at)
          return { id: over.node.id, allowed: drop.kind === 'reparent' }
        })()
      : null

  /** Who else has this node selected. Their colour rings it. */
  const peersOn = (id: string) => peers.filter((p) => p.selected === id)

  return (
    <div className={cn('relative min-h-0 flex-1', className)}>
      <svg
        ref={svgRef}
        role="application"
        aria-label={title}
        tabIndex={0}
        className={cn(
          'bg-muted h-full w-full touch-none outline-none',
          relationFrom ? 'cursor-crosshair' : drag.kind === 'pan' ? 'cursor-grabbing' : 'cursor-grab',
        )}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => setDrag({ kind: 'none' })}
        onWheel={onWheel}
        onKeyDown={onKeyDown}
        onDoubleClick={(e) => {
          const hit = nodeAt(placed.nodes, toWorld(pointIn(e), viewport))
          if (hit) startEditing(hit.node.id)
        }}
      >
        <g transform={`translate(${viewport.x} ${viewport.y}) scale(${viewport.zoom})`}>
          {/* Edges first, so a node always draws over its own lines. */}
          <g fill="none" className="stroke-border" strokeWidth={1.5}>
            {placed.nodes.map((p) => {
              const from = p.node.parent ? positionOf(p.node.parent) : placed.root
              return (
                <path
                  key={`e-${p.node.id}`}
                  d={edgePath(from, positionOf(p.node.id), mode === 'radial' ? 'auto' : 'right')}
                />
              )
            })}
          </g>

          {/* What a hierarchy edge is called, when somebody named it. */}
          <g className="fill-muted-foreground" fontSize={10} textAnchor="middle">
            {placed.nodes
              .filter((p) => p.node.edge_label)
              .map((p) => {
                const from = p.node.parent
                  ? centreOf(p.node.parent)
                  : { x: placed.root.x + NODE_WIDTH / 2, y: placed.root.y + NODE_HEIGHT / 2 }
                const to = centreOf(p.node.id)
                return (
                  <text key={`el-${p.node.id}`} x={(from.x + to.x) / 2} y={(from.y + to.y) / 2 - 4}>
                    {p.node.edge_label}
                  </text>
                )
              })}
          </g>

          {/* Relations: dashed, so they read as "not the tree" at a glance, which
              is the only thing about them a reader has to get right. */}
          <g fill="none" strokeWidth={1.5} strokeDasharray="5 4" className="stroke-ring">
            {relationships.map((r) =>
              byId.has(r.from) && byId.has(r.to) ? (
                <path
                  key={`r-${r.id}`}
                  d={edgePath(positionOf(r.from), positionOf(r.to), 'auto')}
                />
              ) : null,
            )}
          </g>
          <g className="fill-ring" fontSize={10.5} textAnchor="middle">
            {relationships.map((r) => {
              if (!byId.has(r.from) || !byId.has(r.to) || !r.label) return null
              const a = centreOf(r.from)
              const b = centreOf(r.to)
              return (
                <text key={`rl-${r.id}`} x={(a.x + b.x) / 2} y={(a.y + b.y) / 2 - 5}>
                  {r.label}
                </text>
              )
            })}
          </g>

          {/* The root: the map's title, which is not a node and cannot be edited
              here — it is the map, and renaming it belongs with the map. */}
          <g transform={`translate(${placed.root.x} ${placed.root.y})`}>
            <rect
              width={NODE_WIDTH}
              height={NODE_HEIGHT}
              rx={10}
              className="fill-primary stroke-primary"
            />
            <foreignObject width={NODE_WIDTH} height={NODE_HEIGHT}>
              <div className="text-primary-foreground flex h-full items-center justify-center px-3 text-center text-[13px] font-[720]">
                {title}
              </div>
            </foreignObject>
          </g>

          {ordered.map((p) => {
            const at = positionOf(p.node.id)
            const isSelected = selected === p.node.id
            const target = dropTarget?.id === p.node.id ? dropTarget : null
            const watchers = peersOn(p.node.id)
            const hidden = descendantCounts.get(p.node.id) ?? 0
            const folded = collapsed.has(p.node.id)
            // Only the selected card carries its detail. At the 500-node cap
            // every card in full would be unreadable, so the rest keep their
            // marks and the reader still sees where the substance is.
            const width = isSelected ? EXPANDED_WIDTH : NODE_WIDTH
            const height = isSelected ? EXPANDED_HEIGHT : NODE_HEIGHT
            const offset = (NODE_WIDTH - width) / 2
            return (
              <g key={p.node.id} transform={`translate(${at.x} ${at.y})`}>
                {/* A collaborator's ring sits OUTSIDE the box, so it never fights
                    with the selection stroke or hides the text. */}
                {watchers.map((w, i) => (
                  <rect
                    key={`${w.name}-${i}`}
                    x={-3 - i * 3}
                    y={-3 - i * 3}
                    width={NODE_WIDTH + 6 + i * 6}
                    height={NODE_HEIGHT + 6 + i * 6}
                    rx={12}
                    fill="none"
                    stroke={w.color}
                    strokeWidth={2}
                  />
                ))}
                <rect
                  x={offset}
                  width={width}
                  height={height}
                  rx={isSelected ? 12 : cornerRadius(p.node.shape)}
                  className={cn(
                    'fill-card stroke-border',
                    isSelected && 'stroke-ring',
                    relationFrom === p.node.id && 'stroke-ring',
                    target?.allowed && 'stroke-ok',
                    target && !target.allowed && 'stroke-destructive',
                  )}
                  style={p.node.color && !isSelected ? { fill: p.node.color } : undefined}
                  strokeWidth={isSelected || target || relationFrom === p.node.id ? 2 : 1}
                />
                <foreignObject x={offset} width={width} height={height}>
                  <NodeCard
                    node={p.node}
                    expanded={isSelected}
                    canWrite={canWrite}
                    editing={editing === p.node.id}
                    draft={draft}
                    onDraft={setDraft}
                    onCommit={commit}
                    onCancel={() => setEditing(null)}
                    onEdit={() => startEditing(p.node.id)}
                    relations={relationsFor(p.node.id)}
                    titleOf={titleOf}
                    onNotes={onNotes}
                    onFields={onFields}
                    onAddAttachment={onAddAttachment}
                    onRemoveAttachment={onRemoveAttachment}
                    onRemoveRelation={onRemoveRelation}
                    reveal={isSelected ? reveal : null}
                    onRevealed={onRevealed}
                    labels={cardLabels}
                  />
                </foreignObject>

                {/* The fold handle. Only where there is something to fold, only
                    ever for this viewer, and never under the expanded card —
                    ⌘K and Space fold the selected node instead. */}
                {!isSelected && (hidden > 0 || folded) && (
                  <g
                    onPointerDown={(e) => {
                      e.stopPropagation()
                      onToggleCollapse(p.node.id)
                    }}
                    className="cursor-pointer"
                    aria-label={folded ? labels.expand : labels.collapse}
                  >
                    <circle
                      cx={NODE_WIDTH}
                      cy={NODE_HEIGHT}
                      r={9}
                      className="fill-card stroke-border"
                      strokeWidth={1}
                    />
                    <text
                      x={NODE_WIDTH}
                      y={NODE_HEIGHT + 3.5}
                      textAnchor="middle"
                      fontSize={9}
                      className="fill-muted-foreground"
                    >
                      {folded ? hidden || '+' : '−'}
                    </text>
                  </g>
                )}
              </g>
            )
          })}
        </g>
      </svg>

      {nodes.length === 0 && (
        <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center gap-1 text-center">
          <div className="text-foreground text-[15px] font-[680]">{labels.empty}</div>
          <div className="text-muted-foreground text-[13px]">{labels.emptyHint}</div>
        </div>
      )}

      {relationFrom && (
        <div className="bg-card border-border text-foreground pointer-events-none absolute top-3 left-1/2 -translate-x-1/2 rounded-lg border px-3 py-1.5 text-[12px] font-[650]">
          {labels.pickRelationTarget}
        </div>
      )}

      {/* Viewport controls, bottom-right — out of the way of the root. */}
      <div className="absolute right-3 bottom-3 flex gap-1.5">
        <Hint text={mode === 'radial' ? labels.tree : labels.radial}>
          <button
            type="button"
            onClick={() => onMode(mode === 'radial' ? 'tidy' : 'radial')}
            className="bg-card border-border text-muted-foreground hover:text-foreground cursor-pointer rounded-lg border px-2.5 py-1.5 text-[12px] font-[650]"
          >
            {mode === 'radial' ? labels.tree : labels.radial}
          </button>
        </Hint>
        <Hint text={labels.tidy}>
          <button
            type="button"
            onClick={onTidy}
            className="bg-card border-border text-muted-foreground hover:text-foreground cursor-pointer rounded-lg border px-2.5 py-1.5 text-[12px] font-[650]"
          >
            {labels.tidy}
          </button>
        </Hint>
        <Hint text={labels.fit}>
          <button
            type="button"
            onClick={fitAll}
            className="bg-card border-border text-muted-foreground hover:text-foreground cursor-pointer rounded-lg border px-2.5 py-1.5 text-[12px] font-[650]"
          >
            {labels.fit}
          </button>
        </Hint>
        <button
          type="button"
          aria-label={labels.zoomOut}
          onClick={() => zoomBy(1 / 1.2)}
          disabled={viewport.zoom <= MIN_ZOOM}
          className="bg-card border-border text-muted-foreground hover:text-foreground w-8 cursor-pointer rounded-lg border py-1.5 text-[13px] font-[650] disabled:opacity-40"
        >
          −
        </button>
        <button
          type="button"
          aria-label={labels.zoomIn}
          onClick={() => zoomBy(1.2)}
          disabled={viewport.zoom >= MAX_ZOOM}
          className="bg-card border-border text-muted-foreground hover:text-foreground w-8 cursor-pointer rounded-lg border py-1.5 text-[13px] font-[650] disabled:opacity-40"
        >
          +
        </button>
      </div>
    </div>
  )
}
