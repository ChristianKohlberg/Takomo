// The mindmap canvas: pan, zoom, drag, and a keyboard that keeps up with talking.
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
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import {
  DEFAULT_VIEWPORT,
  MAX_ZOOM,
  MIN_ZOOM,
  NODE_HEIGHT,
  NODE_WIDTH,
  edgePath,
  fit,
  layout,
  nodeAt,
  resolveDrop,
  toWorld,
  zoomAt,
  type Point,
  type Viewport,
} from '@/lib/mindmap-layout'
import { cn } from '@/lib/utils'
import type { Mindmap, MindmapNode } from '@/lib/mindmaps'

export interface CanvasLabels {
  /** The root box's caption, when the map has no nodes yet. */
  empty: string
  emptyHint: string
  fit: string
  tidy: string
  zoomIn: string
  zoomOut: string
  /** Announced on the drag target while a drop would be refused. */
  cannotDrop: string
}

export interface CanvasProps {
  mindmap: Mindmap
  nodes: MindmapNode[]
  selected: string | null
  onSelect: (id: string | null) => void
  /** Commit an edited node's text. Called on blur or Escape, not per keystroke. */
  onText: (id: string, text: string) => void
  /** Enter: a sibling after this node. Tab: a child of it. */
  onSibling: (id: string) => void
  onChild: (id: string) => void
  onDelete: (id: string) => void
  onReparent: (id: string, parent: string) => void
  onPlace: (id: string, at: Point) => void
  /** Clear every hand placement and let the layout take over again. */
  onTidy: () => void
  labels: CanvasLabels
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

export function Canvas({
  mindmap,
  nodes,
  selected,
  onSelect,
  onText,
  onSibling,
  onChild,
  onDelete,
  onReparent,
  onPlace,
  onTidy,
  labels,
  className,
}: CanvasProps) {
  const svgRef = useRef<SVGSVGElement | null>(null)
  const [viewport, setViewport] = useState<Viewport>(DEFAULT_VIEWPORT)
  const [drag, setDrag] = useState<Drag>({ kind: 'none' })
  const [editing, setEditing] = useState<string | null>(null)
  const [draft, setDraft] = useState('')
  const fitted = useRef(false)

  const placed = layout(nodes)
  const byId = new Map(placed.nodes.map((p) => [p.node.id, p]))

  // Fit once, when the map first arrives with something in it. Refitting on every
  // change would yank the view out from under somebody who had panned somewhere.
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

  const onPointerDown = (e: React.PointerEvent<SVGSVGElement>) => {
    if (editing) return
    const screen = pointIn(e)
    const world = toWorld(screen, viewport)
    const hit = nodeAt(placed.nodes, world)
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
    if (drag.kind === 'node' && drag.moved) {
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
      if (!node) return
      setDraft(node.text)
      setEditing(id)
    },
    [nodes],
  )

  const commit = useCallback(() => {
    if (editing) onText(editing, draft)
    setEditing(null)
  }, [editing, draft, onText])

  // The keyboard is what makes this keep up with a conversation, so it lives on
  // the canvas rather than only inside a text box: with a node selected, Enter
  // and Tab grow the map without the mouse.
  const onKeyDown = (e: React.KeyboardEvent) => {
    if (editing) return
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
    } else if (e.key === 'Escape') {
      onSelect(null)
    }
  }

  // Focus the textarea when an edit opens, and select it all: the common edit is
  // replacing a first-draft thought, not appending to it.
  const editorRef = useRef<HTMLTextAreaElement | null>(null)
  useEffect(() => {
    if (editing) editorRef.current?.select()
  }, [editing])

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

  const dropTarget =
    drag.kind === 'node' && drag.moved
      ? (() => {
          const node = nodes.find((n) => n.id === drag.id)
          if (!node) return null
          const over = nodeAt(placed.nodes, {
            x: drag.at.x + drag.grab.x,
            y: drag.at.y + drag.grab.y,
          })
          if (!over || over.node.id === drag.id) return null
          const drop = resolveDrop(
            placed.nodes,
            node,
            { x: drag.at.x + drag.grab.x, y: drag.at.y + drag.grab.y },
            drag.at,
          )
          return { id: over.node.id, allowed: drop.kind === 'reparent' }
        })()
      : null

  return (
    <div className={cn('relative min-h-0 flex-1', className)}>
      <svg
        ref={svgRef}
        role="application"
        aria-label={mindmap.title}
        tabIndex={0}
        className={cn(
          'bg-muted h-full w-full touch-none outline-none',
          drag.kind === 'pan' ? 'cursor-grabbing' : 'cursor-grab',
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
                <path key={`e-${p.node.id}`} d={edgePath(from, positionOf(p.node.id))} />
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
                {mindmap.title}
              </div>
            </foreignObject>
          </g>

          {placed.nodes.map((p) => {
            const at = positionOf(p.node.id)
            const isSelected = selected === p.node.id
            const target = dropTarget?.id === p.node.id ? dropTarget : null
            return (
              <g key={p.node.id} transform={`translate(${at.x} ${at.y})`}>
                <rect
                  width={NODE_WIDTH}
                  height={NODE_HEIGHT}
                  rx={9}
                  className={cn(
                    'fill-card stroke-border',
                    isSelected && 'stroke-ring',
                    target?.allowed && 'stroke-ok',
                    target && !target.allowed && 'stroke-destructive',
                  )}
                  strokeWidth={isSelected || target ? 2 : 1}
                />
                <foreignObject width={NODE_WIDTH} height={NODE_HEIGHT}>
                  {editing === p.node.id ? (
                    <textarea
                      ref={editorRef}
                      autoFocus
                      value={draft}
                      onChange={(e) => setDraft(e.target.value)}
                      onBlur={commit}
                      onKeyDown={(e) => {
                        // Enter commits rather than adding a line: a node is a
                        // sentence or two, and the next Enter should make the next
                        // thought. Shift+Enter is the escape hatch.
                        if (e.key === 'Enter' && !e.shiftKey) {
                          e.preventDefault()
                          commit()
                        } else if (e.key === 'Escape') {
                          e.preventDefault()
                          setEditing(null)
                        }
                        e.stopPropagation()
                      }}
                      className="text-foreground h-full w-full resize-none bg-transparent px-2.5 py-1.5 text-[12.5px] leading-snug outline-none"
                    />
                  ) : (
                    <div className="flex h-full flex-col justify-center gap-0.5 px-2.5 py-1.5">
                      <div className="text-foreground line-clamp-2 text-[12.5px] leading-snug">
                        {p.node.text}
                      </div>
                      {/* What this branch became. The badge is the whole reason a
                          map stays useful once the brainstorming is over. */}
                      {p.node.promoted && (
                        <div className="text-muted-foreground truncate font-mono text-[10px]">
                          → {p.node.promoted.kind} {p.node.promoted.id}
                        </div>
                      )}
                    </div>
                  )}
                </foreignObject>
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

      {/* Viewport controls, bottom-right — out of the way of the root, which sits
          left of everything. */}
      <div className="absolute right-3 bottom-3 flex gap-1.5">
        <button
          type="button"
          onClick={onTidy}
          title={labels.tidy}
          className="bg-card border-border text-muted-foreground hover:text-foreground cursor-pointer rounded-lg border px-2.5 py-1.5 text-[12px] font-[650]"
        >
          {labels.tidy}
        </button>
        <button
          type="button"
          onClick={fitAll}
          title={labels.fit}
          className="bg-card border-border text-muted-foreground hover:text-foreground cursor-pointer rounded-lg border px-2.5 py-1.5 text-[12px] font-[650]"
        >
          {labels.fit}
        </button>
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
