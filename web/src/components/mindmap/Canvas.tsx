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
// SELECTING A NODE DOES NOT OPEN IT. Selection highlights it, brings up the pill
// and the `+`, and changes nothing else. It used to expand the card into a
// 300×320 reading panel over its neighbours, so every click on the map threw a
// panel across it whether or not the reader wanted one — which is why every node
// is now drawn at exactly `NODE_WIDTH`×`NODE_HEIGHT` and the geometry in
// `lib/mindmap-layout.ts` is the only thing that decides where anything is.
// Reading a thought properly is `NodeDialog`, reached by the pill, ⌘K and the
// right-click menu.
//
// THE ONE TEXT CARET ON THIS CANVAS IS A TITLE. A node being named draws its
// title as an input, so a new thought is typed straight onto the map instead of
// through a modal; `NodeNameInput` stops every event that would otherwise pan,
// zoom, fold or prune while somebody is typing. Everything else about a thought
// is a field in the dialog.
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import {
  DEFAULT_VIEWPORT,
  centreOn,
  MAX_ZOOM,
  MIN_ZOOM,
  NODE_HEIGHT,
  AFFORDANCE_WIDTH,
  NODE_WIDTH,
  edgePath,
  fit,
  layout,
  nodeAt,
  radialLayout,
  resolveDrop,
  toScreen,
  toWorld,
  zoomAt,
  type Point,
  type Viewport,
} from '@/lib/mindmap-layout'
import { PlusIcon } from 'lucide-react'
import { cn } from '@/lib/utils'
import { trustOf, type FoldSummary, type Trust } from '@/lib/mindmap-lens'
import type { MapNode, Relationship } from '@/lib/mindmap-doc'
import type { DropPayload } from '@/lib/mindmap-attach'
import { Hint } from '@/components/Hint'
import { NodeCard, type NodeCardLabels } from '@/components/mindmap/NodeCard'
import type { NameThen } from '@/components/mindmap/NodeNameInput'
import { NodeMenu, type MenuItem } from '@/components/mindmap/NodeMenu'
import { NodePill, type PillVerb } from '@/components/mindmap/NodePill'

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
  /** The count badge's accessible name. `{n}` is the count. */
  attachments: string
  /** The `+` beside a node. */
  addChild: string
  /** The pill over the selected node, as a toolbar name. */
  nodeActions: string
  /** The right-click menu, as a menu name. */
  nodeMenu: string
  /** Said on the node a file or link is about to be dropped onto. */
  dropHere: string
  /** The trust lens: its toggle, its legend, and its three readings. */
  trustLens: string
  trustLegend: string
  trustConfirmed: string
  trustMachine: string
  trustUnverified: string
  /** Clicking the line to a parent. `{title}` and `{parent}` name both ends. */
  cutEdge: string
  /** The inline title caret: its accessible name, and its placeholder. */
  nameField: string
  nameHint: string
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
  /**
   * The node whose title is being typed right now, or null.
   *
   * Owned one level up because creating a node and naming it are one gesture,
   * and the create half is a document write the canvas does not do.
   */
  naming: string | null
  /** The caret closed with a name in it. `then` is what the key that closed it
   *  asked for next: Enter stays on this thought, Tab goes a level deeper. */
  onNameCommit: (id: string, title: string, then: NameThen) => void
  /** Escape. What that does to the node is the page's decision. */
  onNameCancel: (id: string) => void
  /** Rename in place — F2 and double-click. The one way a title is changed. */
  onRenameNode: (id: string) => void
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
  /** What a card's marks are called. A card is a title and its marks; the whole
   *  of a thought is the dialog, one level up. */
  cardLabels: NodeCardLabels
  relationsFor: (id: string) => Relationship[]
  titleOf: ReadonlyMap<string, string>
  /** The count badge, and the right-click menu's attachment entry. */
  onOpenAttachments: (id: string) => void
  /**
   * Something was dropped onto a node. The canvas reduces the browser's
   * `DataTransfer` to file NAMES and text and hands that on — it never reads a
   * byte, because an attachment here is a pointer and bytes in a shared document
   * are bytes every peer replays on join.
   */
  onAttachDrop: (id: string, payload: DropPayload) => void
  /** The three or four verbs the pill offers over the selected node. */
  pillVerbs: readonly PillVerb[]
  /**
   * What right-clicking a node offers — a function of the node, not of the
   * selection, because right-click opens the menu WITHOUT selecting anything.
   * Selecting is what brings the pill up, and a menu that selected as a side
   * effect would be doing two things when one was asked for.
   */
  menuItemsFor: (id: string) => readonly MenuItem[]
  /** Runs a pill verb or a menu entry. Both are command ids; `target` names the
   *  node a menu entry acts on, since that node need not be the selected one. */
  onRunVerb: (id: string, target?: string) => void
  /**
   * Asks from the page, each cleared the moment it is honoured: bring a node into
   * the middle of the view, fit the map, or take the keyboard back.
   */
  centreNode: string | null
  onCentred: () => void
  /** A timestamp, so asking twice asks twice. Null while nothing is pending. */
  fitRequest: number | null
  onFitted: () => void
  /**
   * Give the canvas the keyboard back — what the page asks after a dialog closes.
   * Without it, Enter and Tab stop growing the map the moment somebody names a
   * node, because the focus went to the dialog and never came back.
   */
  focusRequest: number | null
  onFocused: () => void
  /** What each folded branch is holding. Only this viewer's folds have one, and
   *  the canvas cannot compute it — the hidden nodes are not among `nodes`. */
  foldSummaryOf: (id: string) => FoldSummary | null
  /** Tint every node by how confident we are in it. A lens, off by default. */
  trustLens: boolean
  onTrustLens: (on: boolean) => void
  /** A double-click into empty space: a thought that does not know where it goes
   *  yet, pinned where it was dropped. World coordinates. */
  onCreateAt: (at: Point) => void
  /** Clicking the line between a node and its parent. The page asks twice before
   *  anything is cut, so this only OFFERS the cut. */
  onCutEdge: (childId: string) => void
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
/**
 * The three tints of the trust lens.
 *
 * Deliberately a fill AND a stroke rather than a badge: the question the lens
 * answers — "what in here has nobody looked at?" — is asked of the whole map at
 * once, and a mark you have to read node by node does not answer it. The words
 * are on the card and in the legend, so the reading is never colour alone.
 */
const TRUST_FILL: Record<Trust, string> = {
  confirmed: 'fill-emerald-100 stroke-emerald-500 dark:fill-emerald-950',
  machine: 'fill-rose-100 stroke-rose-500 dark:fill-rose-950',
  unverified: 'fill-amber-100 stroke-amber-500 dark:fill-amber-950',
}

/** The legend's swatches, in the same order the lens reads. */
const TRUST_SWATCH: Record<Trust, string> = {
  confirmed: 'bg-emerald-300 dark:bg-emerald-700',
  machine: 'bg-rose-300 dark:bg-rose-700',
  unverified: 'bg-amber-300 dark:bg-amber-700',
}

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
  naming,
  onNameCommit,
  onNameCancel,
  onRenameNode,
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
  onOpenAttachments,
  onAttachDrop,
  pillVerbs,
  menuItemsFor,
  onRunVerb,
  centreNode,
  onCentred,
  fitRequest,
  onFitted,
  focusRequest,
  onFocused,
  foldSummaryOf,
  trustLens,
  onTrustLens,
  onCreateAt,
  onCutEdge,
  className,
}: CanvasProps) {
  const svgRef = useRef<SVGSVGElement | null>(null)
  const [viewport, setViewport] = useState<Viewport>(DEFAULT_VIEWPORT)
  const [drag, setDrag] = useState<Drag>({ kind: 'none' })
  // The node under the pointer. Only used to reveal the `+`, and only ever set
  // when it CHANGES — a 500-node map re-rendering on every mousemove is the one
  // way a hover affordance could cost more than it is worth.
  const [hovered, setHovered] = useState<string | null>(null)
  // The open right-click menu: where it is drawn, in container pixels, and which
  // node it acts on. The node is carried here rather than taken from `selected`
  // because opening this menu selects nothing.
  const [menu, setMenu] = useState<{ at: Point; id: string } | null>(null)
  // The node a file or link is hovering over, mid-drag.
  const [dropOver, setDropOver] = useState<string | null>(null)
  const fitted = useRef(false)

  const placed = mode === 'radial' ? radialLayout(nodes) : layout(nodes)
  const byId = new Map(placed.nodes.map((p) => [p.node.id, p]))
  // The node being named is drawn LAST. Every card is the same size now, so
  // nothing overlaps by design — but a hand-placed thought can sit on top of
  // another, and in SVG there is no z-index, so paint order is the only way to
  // keep an open caret from ending up underneath one.
  const ordered = [...placed.nodes].sort(
    (a, b) => Number(a.node.id === naming) - Number(b.node.id === naming),
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
   * The node under a point.
   *
   * Every card is drawn at its layout box now, so this is the layout's own
   * hit-test and nothing else — the special case for a selected card wider than
   * its box went with the expanded card.
   */
  const hitAt = (world: Point) => nodeAt(placed.nodes, world)

  const onPointerDown = (e: React.PointerEvent<SVGSVGElement>) => {
    // Only the primary button does anything on the map. The right button ran the
    // whole select-and-begin-drag path before `contextmenu` ever fired, so a
    // right-click both opened a node and opened a menu over it.
    if (e.button !== 0) return
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
    if (drag.kind === 'none') {
      // Strict first, so a point genuinely inside another node belongs to that
      // node; only then the affordance strip, so the `+` stays reachable.
      const world = toWorld(pointIn(e), viewport)
      const over =
        (nodeAt(placed.nodes, world) ?? nodeAt(placed.nodes, world, AFFORDANCE_WIDTH))?.node.id ??
        null
      if (over !== hovered) setHovered(over)
      return
    }
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

  /**
   * Open the title caret on a node.
   *
   * Gated on `canWrite`, unlike the dialog: a caret is a write, and a read-only
   * token must never get one from any entrance.
   */
  const rename = (id: string) => {
    if (canWrite) onRenameNode(id)
  }

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

  // The dialog took the focus with it when it opened; this is how it comes back,
  // so the map keyboard works again without a click into the canvas first.
  useEffect(() => {
    if (focusRequest === null) return
    svgRef.current?.focus()
    onFocused()
  }, [focusRequest, onFocused])

  // The keyboard is what makes this keep up with a conversation, so it lives on
  // the canvas rather than only inside a text box: with a node selected, Enter
  // and Tab grow the map without the mouse.
  const onKeyDown = (e: React.KeyboardEvent) => {
    // While a title is being typed the caret owns the keyboard — it stops every
    // key itself, and this is the belt to that pair of braces.
    if (naming) return
    if (e.key === 'Escape' && relationFrom) {
      e.preventDefault()
      onCancelRelation()
      return
    }
    if (!selected) return
    // A menu you can only open with a right-click is a set of commands a
    // keyboard does not have. Shift+F10 and the ContextMenu key are what every
    // other application answers to, so they open the same menu, anchored on the
    // selected node rather than on a pointer that is not there.
    if (e.key === 'ContextMenu' || (e.key === 'F10' && e.shiftKey)) {
      e.preventDefault()
      openMenuFor(selected)
      return
    }
    if (e.key === 'Enter') {
      e.preventDefault()
      onSibling(selected)
    } else if (e.key === 'Tab') {
      e.preventDefault()
      onChild(selected)
    } else if (e.key === 'F2') {
      e.preventDefault()
      rename(selected)
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

  /**
   * Keep the menu inside the canvas instead of half off its right or bottom edge.
   *
   * The size is approximated rather than measured, deliberately: measuring means
   * rendering it off-screen first and moving it, which is a visible jump, and
   * being a few pixels out on the clamp costs nothing.
   */
  const clampMenu = (screen: Point, rows: number): Point => {
    const box = svgRef.current?.getBoundingClientRect()
    const width = 232
    const height = 34 * Math.max(rows, 1) + 16
    return {
      x: Math.max(4, Math.min(screen.x, (box?.width ?? width) - width - 4)),
      y: Math.max(4, Math.min(screen.y, (box?.height ?? height) - height - 4)),
    }
  }

  /** Open the menu anchored under a node — what the keyboard path uses, since it
   *  has no pointer to anchor on. */
  const openMenuFor = (id: string) => {
    const at = positionOf(id)
    setMenu({
      id,
      at: clampMenu(
        toScreen({ x: at.x + NODE_WIDTH / 2, y: at.y + NODE_HEIGHT }, viewport),
        menuItemsFor(id).length,
      ),
    })
  }

  const closeMenu = () => {
    // Give the keyboard back to the canvas only if the menu is what had it. A
    // press somewhere else also closes this, and stealing the focus away from
    // wherever that press landed is worse than leaving it alone.
    const fromMenu = document.activeElement?.closest?.('[role="menu"]') != null
    setMenu(null)
    if (fromMenu) svgRef.current?.focus()
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

  /** The node a drag is over, or null. Shared by the dragover and drop paths so
   *  the highlight and the write can never disagree about the target. */
  const nodeUnderDrag = (e: { clientX: number; clientY: number }) =>
    hitAt(toWorld(pointIn(e), viewport))?.node.id ?? null

  return (
    <div
      className={cn('relative min-h-0 flex-1', className)}
      // The default MUST be prevented on the whole canvas, not just on a node:
      // a file dropped anywhere else navigates the browser to it, which throws
      // away the map, the connection and whatever anyone was typing.
      onDragOver={(e) => {
        e.preventDefault()
        if (!canWrite) return
        e.dataTransfer.dropEffect = 'copy'
        const over = nodeUnderDrag(e)
        if (over !== dropOver) setDropOver(over)
      }}
      onDragLeave={(e) => {
        // Moving onto a child of this container is not leaving it.
        if (e.currentTarget.contains(e.relatedTarget as Node | null)) return
        setDropOver(null)
      }}
      onDrop={(e) => {
        e.preventDefault()
        setDropOver(null)
        if (!canWrite) return
        const target = nodeUnderDrag(e)
        if (!target) return
        onAttachDrop(target, {
          // Names only. The bytes are deliberately never read.
          files: [...e.dataTransfer.files].map((f) => ({ name: f.name })),
          text: e.dataTransfer.getData('text/uri-list') || e.dataTransfer.getData('text/plain'),
        })
      }}
    >
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
        onPointerLeave={() => setHovered(null)}
        onContextMenu={(e) => {
          // The browser's own menu is never the right answer over a canvas: it
          // offers "save image as" for a thought somebody wrote.
          e.preventDefault()
          const screen = pointIn(e)
          const hit = hitAt(toWorld(screen, viewport))
          if (!hit) {
            setMenu(null)
            return
          }
          // The menu, and ONLY the menu. Selecting here would bring the pill up
          // under a menu nobody asked to have a node opened by.
          setMenu({
            id: hit.node.id,
            at: clampMenu(screen, menuItemsFor(hit.node.id).length),
          })
        }}
        onDoubleClick={(e) => {
          const world = toWorld(pointIn(e), viewport)
          const hit = hitAt(world)
          if (hit) {
            // Double-click is rename, not open: the title is the one thing typed
            // on the map, and everything else about a thought is the dialog.
            rename(hit.node.id)
            return
          }
          // Empty space: a loose thought, pinned where it was dropped and opened
          // straight into its title. You do not always know where a thought
          // goes, and forcing a parent is wrong for the ten minutes a brainstorm
          // is for. Centred on the cursor, because that is where it was aimed.
          if (!canWrite) return
          // The root box is the MAP, not a node, and it is not in `placed.nodes`
          // for `nodeAt` to have found — so it has to be excluded by hand, or a
          // double-click on the title drops a thought on top of it.
          const onRoot =
            world.x >= placed.root.x &&
            world.x <= placed.root.x + NODE_WIDTH &&
            world.y >= placed.root.y &&
            world.y <= placed.root.y + NODE_HEIGHT
          if (onRoot) return
          onCreateAt({ x: world.x - NODE_WIDTH / 2, y: world.y - NODE_HEIGHT / 2 })
        }}
      >
        <g transform={`translate(${viewport.x} ${viewport.y}) scale(${viewport.zoom})`}>
          {/* Edges first, so a node always draws over its own lines.

              A line to a PARENT is clickable, and the click offers to cut it —
              the child becomes a first-ring thought and nothing is removed. The
              hit target is a fat transparent stroke over the visible hairline,
              because a 1.5px line is not something a pointer can be asked to
              find. A first-ring node's line goes to the map itself rather than
              to a node, so there is nothing there to cut. */}
          <g fill="none" strokeWidth={1.5}>
            {placed.nodes.map((p) => {
              const from = p.node.parent ? positionOf(p.node.parent) : placed.root
              const d = edgePath(from, positionOf(p.node.id), mode === 'radial' ? 'auto' : 'right')
              const cuttable = canWrite && p.node.parent !== null
              if (!cuttable) return <path key={`e-${p.node.id}`} className="stroke-border" d={d} />
              return (
                <g
                  key={`e-${p.node.id}`}
                  className="group cursor-pointer"
                  onPointerDown={(e) => e.stopPropagation()}
                  onClick={() => onCutEdge(p.node.id)}
                >
                  <title>
                    {labels.cutEdge
                      .replace('{title}', p.node.title)
                      .replace('{parent}', titleOf.get(p.node.parent as string) ?? '')}
                  </title>
                  <path d={d} stroke="transparent" strokeWidth={14} />
                  <path className="stroke-border group-hover:stroke-destructive" d={d} />
                </g>
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
          <g fill="none" strokeWidth={1.5}>
            {relationships.map((r) => {
              if (!byId.has(r.from) || !byId.has(r.to)) return null
              // The line from a question to what it questions is a relation like
              // any other — that is what let questions exist with no new field
              // anywhere — but it is not read like one, so it is not drawn like
              // one either.
              const asks =
                byId.get(r.from)?.node.kind === 'question' ||
                byId.get(r.to)?.node.kind === 'question'
              return (
                <path
                  key={`r-${r.id}`}
                  d={edgePath(positionOf(r.from), positionOf(r.to), 'auto')}
                  strokeDasharray={asks ? '2 5' : '5 4'}
                  className={asks ? 'stroke-violet-500' : 'stroke-ring'}
                />
              )
            })}
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
            // A caret only ever opens on a node this token may write to, and it
            // is checked here as well as at every entrance: a read-only token
            // must not get one by any route.
            const isNaming = naming === p.node.id && canWrite
            const target = dropTarget?.id === p.node.id ? dropTarget : null
            const watchers = peersOn(p.node.id)
            const hidden = descendantCounts.get(p.node.id) ?? 0
            const folded = collapsed.has(p.node.id)
            const attachments = p.node.attachments.length
            const dropping = dropOver === p.node.id
            // The `+` is on hover OR selection, never permanently: 500 nodes
            // with a plus sign each is 500 things to look past.
            const showAdd = canWrite && (hovered === p.node.id || isSelected)
            const isQuestion = p.node.kind === 'question'
            // One value, used by the rect and by its inset — they must agree or
            // the border lands half a pixel off its own box.
            const strokeW = isSelected || target || relationFrom === p.node.id ? 2 : 1
            // The lens is a lens: while it is on it OVERRIDES a hand-picked
            // colour, because a map half tinted by confidence and half by
            // somebody's palette answers neither question.
            const trust = trustLens && !isQuestion ? trustOf(p.node) : null
            const fold = folded ? foldSummaryOf(p.node.id) : null
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
                {/* Inset by half the stroke, so the border is drawn INSIDE the
                    node's own box.
                    An SVG stroke straddles the path, so a rect flush at
                    0..NODE_WIDTH puts half its width outside — and that half is
                    what an ancestor's clip, the svg viewport at the edge of the
                    canvas, or a sibling drawn on top takes away. Hence borders
                    that looked cut on their outer edge. Inside the box, nothing
                    can cut them. */}
                <rect
                  x={strokeW / 2}
                  y={strokeW / 2}
                  width={NODE_WIDTH - strokeW}
                  height={NODE_HEIGHT - strokeW}
                  // A question is squarer than a thought: it is a different kind
                  // of thing on the map, and shape says so before colour does.
                  rx={isQuestion ? 4 : cornerRadius(p.node.shape)}
                  className={cn(
                    'fill-card stroke-border',
                    trust && TRUST_FILL[trust],
                    isQuestion && 'fill-violet-50 stroke-violet-400 dark:fill-violet-950',
                    isSelected && 'stroke-ring',
                    relationFrom === p.node.id && 'stroke-ring',
                    target?.allowed && 'stroke-ok',
                    target && !target.allowed && 'stroke-destructive',
                  )}
                  style={
                    p.node.color && !trust && !isQuestion ? { fill: p.node.color } : undefined
                  }
                  strokeWidth={strokeW}
                />
                <foreignObject width={NODE_WIDTH} height={NODE_HEIGHT}>
                  <NodeCard
                    node={p.node}
                    relations={relationsFor(p.node.id)}
                    fold={fold}
                    trust={trust}
                    naming={
                      isNaming
                        ? {
                            onCommit: (text, then) => onNameCommit(p.node.id, text, then),
                            onCancel: () => onNameCancel(p.node.id),
                            labels: { field: labels.nameField, hint: labels.nameHint },
                          }
                        : null
                    }
                    labels={cardLabels}
                  />
                </foreignObject>

                {/* Where a dropped file or link would land. Outside the box so
                    it cannot be mistaken for the selection stroke, which means
                    something else entirely. */}
                {dropping && (
                  <rect
                    x={-5}
                    y={-5}
                    width={NODE_WIDTH + 10}
                    height={NODE_HEIGHT + 10}
                    rx={14}
                    fill="none"
                    strokeWidth={2}
                    strokeDasharray="5 4"
                    className="stroke-ok"
                  >
                    <title>{labels.dropHere}</title>
                  </rect>
                )}

                {/* The attachment badge. On EVERY node that has one, not only
                    the selected one — it is how you see there is something
                    there at all. Clicking it opens the manager. */}
                {attachments > 0 && (
                  <foreignObject x={NODE_WIDTH - 54} y={-11} width={56} height={22}>
                    <div className="pointer-events-none flex h-full items-center justify-end">
                      <button
                        type="button"
                        aria-label={labels.attachments.replace('{n}', String(attachments))}
                        title={labels.attachments.replace('{n}', String(attachments))}
                        onPointerDown={(e) => e.stopPropagation()}
                        onKeyDown={(e) => e.stopPropagation()}
                        onClick={() => onOpenAttachments(p.node.id)}
                        className="bg-foreground text-background pointer-events-auto cursor-pointer rounded-full px-2 py-0.5 font-mono text-[10px] leading-none shadow-sm"
                      >
                        ⎘ {attachments}
                      </button>
                    </div>
                  </foreignObject>
                )}

                {/* Add a child, right where the child will appear.
                    It used to be a 20px muted circle holding a text "+", one
                    pixel off the node's edge — the same weight as the node's own
                    border, so it read as a speck rather than a control. It is
                    the primary verb on this surface and now looks like one:
                    accent-filled, 28px (a real pointer target), and overlapping
                    the node's edge so it belongs to that node rather than
                    floating between two. Centred on that edge, so its far side
                    lands at NODE_WIDTH + 14 — inside `AFFORDANCE_WIDTH` (30),
                    which is the padding `nodeAt` uses to keep the button alive
                    as the pointer travels to it. A button reaching past that
                    padding would unmount as you approached, which is a bug this
                    surface has had once already. */}
                {showAdd && (
                  <foreignObject x={NODE_WIDTH - 14} y={NODE_HEIGHT / 2 - 18} width={36} height={36}>
                    <div className="pointer-events-none flex h-full items-center">
                      <button
                        type="button"
                        aria-label={labels.addChild}
                        title={labels.addChild}
                        onPointerDown={(e) => e.stopPropagation()}
                        onKeyDown={(e) => e.stopPropagation()}
                        onClick={() => onChild(p.node.id)}
                        className="bg-primary text-primary-foreground ring-card hover:bg-primary/90 focus-visible:ring-ring pointer-events-auto flex h-7 w-7 cursor-pointer items-center justify-center rounded-full shadow-md ring-2 focus-visible:ring-4 focus-visible:outline-none"
                      >
                        <PlusIcon className="size-4" aria-hidden="true" />
                      </button>
                    </div>
                  </foreignObject>
                )}

                {/* The verbs, only on selection, in the margin above the node.
                    Not while it is being named: the caret is one thought's worth
                    of attention and a toolbar over it is the rest of them. */}
                {isSelected && !isNaming && pillVerbs.length > 0 && (
                  <foreignObject x={0} y={-38} width={NODE_WIDTH} height={34}>
                    <div className="pointer-events-none flex h-full items-end justify-center">
                      <NodePill
                        className="pointer-events-auto"
                        verbs={pillVerbs}
                        onRun={(verb) => onRunVerb(verb)}
                        ariaLabel={labels.nodeActions}
                      />
                    </div>
                  </foreignObject>
                )}

                {/* The fold handle. Only where there is something to fold, and
                    only ever for this viewer. It is drawn on the selected node
                    too, now that selecting one no longer covers its corner. */}
                {(hidden > 0 || folded) && (
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

      {menu && menuItemsFor(menu.id).length > 0 && (
        <NodeMenu
          items={menuItemsFor(menu.id)}
          at={menu.at}
          ariaLabel={labels.nodeMenu}
          onRun={(verb) => {
            const target = menu.id
            closeMenu()
            onRunVerb(verb, target)
          }}
          onClose={closeMenu}
        />
      )}

      {/* The legend. Present only while the lens is, because a permanent key to
          a decoration nobody asked for is exactly what a lens is not. */}
      {trustLens && (
        <div className="bg-card border-border absolute bottom-3 left-3 flex max-w-[calc(100vw-1.5rem)] flex-col gap-1 rounded-lg border px-2.5 py-2 text-[11px]">
          <span className="text-muted-foreground text-[10px] font-[650]">
            {labels.trustLegend}
          </span>
          {(
            [
              ['confirmed', labels.trustConfirmed],
              ['unverified', labels.trustUnverified],
              ['machine', labels.trustMachine],
            ] as const
          ).map(([kind, label]) => (
            <span key={kind} className="text-foreground flex items-center gap-1.5">
              <span className={cn('h-2.5 w-2.5 shrink-0 rounded-sm', TRUST_SWATCH[kind])} />
              {label}
            </span>
          ))}
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
        <Hint text={labels.trustLens}>
          <button
            type="button"
            aria-pressed={trustLens}
            onClick={() => onTrustLens(!trustLens)}
            className={cn(
              'bg-card border-border text-muted-foreground hover:text-foreground cursor-pointer rounded-lg border px-2.5 py-1.5 text-[12px] font-[650]',
              trustLens && 'border-ring text-foreground',
            )}
          >
            ◍
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
