// The workflow canvas: states as boxes, transitions as arrows.
//
// Hand-rolled SVG. There is no graph library here and cannot be one — the CSP
// forbids external scripts and nothing suitable is in package.json — so nodes,
// edges, hit-testing and dragging are all local. That is a cost, but a small
// one at this size: a workflow has a handful of states, and the interactions
// worth supporting are "move a box" and "draw an arrow".
//
// Pointer events, not mouse events. `onPointerDown` + `setPointerCapture` is one
// code path for mouse, trackpad, touch and pen, and capture is what makes a drag
// survive the pointer leaving the node — with mouse events, dragging quickly
// drops the box the moment the cursor outruns it.
import { useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import type { Layout, WfTransition, WorkflowDoc } from '@/lib/workflows'
import { Card } from '@/components/ui/card'
import { NODE_H, NODE_W, edgeAnchors, layoutExtent } from './layout'

/**
 * Fill and border per category, so the picture carries the meaning the board
 * already gives these words.
 *
 * Inline `style`, not Tailwind `fill-[color:var(--x)]` utilities. Two reasons,
 * both learned the hard way here: those arbitrary utilities did not resolve at
 * all, and the token names are not what a shadcn palette would suggest —
 * `--muted` in tokens.css is a TEXT colour (#6e7c88), so using it as a fill
 * painted every node slate grey and one of them solid black.
 */
const CATEGORY_FILL: Record<string, { fill: string; stroke: string }> = {
  todo: { fill: 'var(--col-bg)', stroke: 'var(--border)' },
  in_progress: { fill: 'var(--sel)', stroke: 'var(--sel-border)' },
  review: { fill: 'var(--chip-bg)', stroke: 'var(--sel-border)' },
  blocked: { fill: 'var(--nfbg)', stroke: 'var(--nfbd)' },
  done: { fill: 'var(--okbg)', stroke: 'var(--okbd)' },
  cancelled: { fill: 'var(--col-bg)', stroke: 'var(--border)' },
}

export interface CanvasSelection {
  kind: 'state' | 'transition'
  /** State id, or the index of the transition in the document. */
  key: string | number
}

export interface CanvasProps {
  wf: WorkflowDoc
  layout: Layout
  selection: CanvasSelection | null
  onSelect: (s: CanvasSelection | null) => void
  /** A node was dragged to a new position. */
  onMove: (id: string, pos: { x: number; y: number }) => void
  /** A new edge was drawn from one state to another. */
  onConnect: (from: string, to: string) => void
  readOnly?: boolean
  labels: { initial: string; claimable: string; terminal: string; hint: string }
}

export function Canvas({
  wf,
  layout,
  selection,
  onSelect,
  onMove,
  onConnect,
  readOnly,
  labels,
}: CanvasProps) {
  const svgRef = useRef<SVGSVGElement | null>(null)
  // A drag in progress: either moving a node, or pulling a new edge out of one.
  const [drag, setDrag] = useState<
    | { mode: 'move'; id: string; dx: number; dy: number }
    | { mode: 'connect'; from: string; x: number; y: number }
    | null
  >(null)

  const { width, height } = layoutExtent(layout)

  /** Pointer position in SVG user space, which is what the layout is in. */
  function toLocal(e: ReactPointerEvent): { x: number; y: number } {
    const rect = svgRef.current?.getBoundingClientRect()
    return {
      x: e.clientX - (rect?.left ?? 0),
      y: e.clientY - (rect?.top ?? 0),
    }
  }

  function startMove(e: ReactPointerEvent, id: string) {
    if (readOnly) return
    e.stopPropagation()
    const p = toLocal(e)
    const at = layout[id] ?? { x: 0, y: 0 }
    ;(e.target as Element).setPointerCapture?.(e.pointerId)
    setDrag({ mode: 'move', id, dx: p.x - at.x, dy: p.y - at.y })
    onSelect({ kind: 'state', key: id })
  }

  function startConnect(e: ReactPointerEvent, id: string) {
    if (readOnly) return
    e.stopPropagation()
    const p = toLocal(e)
    ;(e.target as Element).setPointerCapture?.(e.pointerId)
    setDrag({ mode: 'connect', from: id, x: p.x, y: p.y })
  }

  function onPointerMove(e: ReactPointerEvent) {
    if (!drag) return
    const p = toLocal(e)
    if (drag.mode === 'move') {
      // Clamped at zero: a node dragged past the top-left would sit outside the
      // SVG's extent, where it cannot be scrolled to and so cannot be recovered.
      onMove(drag.id, { x: Math.max(0, p.x - drag.dx), y: Math.max(0, p.y - drag.dy) })
    } else {
      setDrag({ ...drag, x: p.x, y: p.y })
    }
  }

  function onPointerUp(e: ReactPointerEvent) {
    if (drag?.mode === 'connect') {
      const p = toLocal(e)
      const target = wf.states.find((s) => {
        const at = layout[s.id]
        return (
          at &&
          p.x >= at.x &&
          p.x <= at.x + NODE_W &&
          p.y >= at.y &&
          p.y <= at.y + NODE_H
        )
      })
      // A self-edge is not a transition the server would accept, and dropping on
      // empty space means "never mind" rather than "connect to nothing".
      if (target && target.id !== drag.from) onConnect(drag.from, target.id)
    }
    setDrag(null)
  }

  const edgeKey = (t: WfTransition, i: number) => `${t.from}->${t.to}-${i}`

  return (
    <Card className="overflow-auto py-0">
      <svg
        ref={svgRef}
        width={Math.max(width, 320)}
        height={Math.max(height, 220)}
        className="block touch-none select-none"
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => setDrag(null)}
        onClick={() => onSelect(null)}
        role="application"
        aria-label={labels.hint}
      >
        <defs>
          <marker
            id="wf-arrow"
            viewBox="0 0 10 10"
            refX="9"
            refY="5"
            markerWidth="6"
            markerHeight="6"
            orient="auto-start-reverse"
          >
            <path d="M 0 0 L 10 5 L 0 10 z" style={{ fill: 'var(--muted)' }} />
          </marker>
        </defs>

        {/* Edges first so nodes paint over their endpoints. */}
        {(wf.transitions ?? []).map((t, i) => {
          const a = layout[t.from]
          const b = layout[t.to]
          if (!a || !b) return null
          const { x1, y1, x2, y2 } = edgeAnchors(a, b)
          const selected = selection?.kind === 'transition' && selection.key === i
          const gated = (t.requires ?? []).length > 0
          const mx = (x1 + x2) / 2
          const my = (y1 + y2) / 2
          return (
            <g key={edgeKey(t, i)}>
              {/* A wide transparent line under the visible one: a 1.5px stroke
                  is far below any reasonable hit target, especially on touch. */}
              <line
                x1={x1}
                y1={y1}
                x2={x2}
                y2={y2}
                stroke="transparent"
                strokeWidth={14}
                className="cursor-pointer"
                onClick={(e) => {
                  e.stopPropagation()
                  onSelect({ kind: 'transition', key: i })
                }}
              />
              <line
                x1={x1}
                y1={y1}
                x2={x2}
                y2={y2}
                markerEnd="url(#wf-arrow)"
                className="pointer-events-none"
                style={{ stroke: selected ? 'var(--accent2)' : 'var(--muted2)' }}
                strokeWidth={selected ? 2.5 : 1.5}
                strokeDasharray={gated ? '5 3' : undefined}
              />
              {gated && (
                <circle
                  cx={mx}
                  cy={my}
                  r={3.5}
                  className="pointer-events-none"
                  style={{ fill: 'var(--accent2)' }}
                />
              )}
            </g>
          )
        })}

        {/* The edge being drawn. */}
        {drag?.mode === 'connect' && layout[drag.from] && (
          <line
            x1={(layout[drag.from]?.x ?? 0) + NODE_W}
            y1={(layout[drag.from]?.y ?? 0) + NODE_H / 2}
            x2={drag.x}
            y2={drag.y}
            className="pointer-events-none"
            style={{ stroke: 'var(--accent2)' }}
            strokeWidth={2}
            strokeDasharray="4 4"
          />
        )}

        {wf.states.map((s) => {
          const at = layout[s.id]
          if (!at) return null
          const selected = selection?.kind === 'state' && selection.key === s.id
          const isInitial = wf.initial === s.id
          return (
            <g key={s.id} transform={`translate(${at.x}, ${at.y})`}>
              <rect
                width={NODE_W}
                height={NODE_H}
                rx={10}
                className={readOnly ? 'cursor-default' : 'cursor-grab'}
                style={{
                  fill: (CATEGORY_FILL[s.category] ?? CATEGORY_FILL.todo)!.fill,
                  stroke: selected
                    ? 'var(--accent2)'
                    : (CATEGORY_FILL[s.category] ?? CATEGORY_FILL.todo)!.stroke,
                }}
                strokeWidth={selected ? 2.5 : 1.5}
                onPointerDown={(e) => startMove(e, s.id)}
                onClick={(e) => {
                  e.stopPropagation()
                  onSelect({ kind: 'state', key: s.id })
                }}
              />
              <text
                x={12}
                y={23}
                className="pointer-events-none font-mono text-[12.5px] font-[650]"
                style={{ fill: 'var(--text)' }}
              >
                {s.id.length > 18 ? s.id.slice(0, 17) + '…' : s.id}
              </text>
              <text
                x={12}
                y={40}
                className="pointer-events-none text-[10.5px]"
                style={{ fill: 'var(--muted)' }}
              >
                {s.category}
                {s.claimable ? ` · ${labels.claimable}` : ''}
                {s.terminal ? ` · ${labels.terminal}` : ''}
              </text>
              {isInitial && (
                <text
                  x={NODE_W - 10}
                  y={16}
                  textAnchor="end"
                  className="pointer-events-none text-[9.5px] font-bold tracking-wider uppercase"
                  style={{ fill: 'var(--accent2)' }}
                >
                  {labels.initial}
                </text>
              )}
              {/* The connect handle. Deliberately on the right edge, where a
                  forward edge leaves from. */}
              {!readOnly && (
                <circle
                  cx={NODE_W}
                  cy={NODE_H / 2}
                  r={7}
                  className="cursor-crosshair"
                  style={{ fill: 'var(--accent2)', stroke: 'var(--panel)' }}
                  strokeWidth={2}
                  onPointerDown={(e) => startConnect(e, s.id)}
                />
              )}
            </g>
          )
        })}
      </svg>
    </Card>
  )
}
