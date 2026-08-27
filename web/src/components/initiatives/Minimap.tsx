// The minimap: the whole project as one picture — the project, its initiatives,
// and the epics filed under each.
//
// The explorer answers "which document", and it answers it well because folders
// are how a reader files things. What it cannot answer is "where is the work" —
// an initiative never closes, so its own status says nothing about progress, and
// the versions that DO close are one level further down. This is that level,
// drawn all at once rather than one selected lane at a time.
//
// One colour decision, deliberately only one: an epic whose work is finished is
// green, and every other epic is neutral. A map that colours by state ends up
// with six colours nobody remembers, and the question a map is opened to answer
// is "what is left".
//
// Geometry lives in `lib/initiative-map.ts` as pure functions, for the reason
// the mindmap canvas gives: jsdom has no layout engine, so nothing here could be
// proven by a component test.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  NODE_H,
  byKey,
  edgePath,
  isComplete,
  layoutMap,
  type LaneWithEpics,
  type MapNode,
} from '@/lib/initiative-map'
import type { RoadmapEpic } from '@/lib/roadmap'
import { cn } from '@/lib/utils'
import { Hint } from '@/components/Hint'

export interface MinimapLabels {
  /** Caption over the map. */
  heading: string
  /** Shown instead of the map when no single project is selected. */
  needProject: string
  needProjectHint: string
  /** Shown when the project has no initiatives at all. */
  empty: string
  emptyHint: string
  /** The synthetic lane holding epics no initiative claims. */
  unfiled: string
  unfiledHint: string
  /** Accessible name of the go-to menu, and its entries. */
  goTo: string
  openDocument: string
  openVerification: string
  versions: string
  /** Statistics panel. */
  statistics: string
  close: string
  done: string
  ready: string
  backlog: string
  awaiting: string
  state: string
  priority: string
  openOnBoard: string
  noWork: string
  /** Zoom controls. */
  zoomIn: string
  zoomOut: string
  /** Legend. */
  complete: string
  /** Hint under an epic box, and the empty statistics pane. */
  pickEpic: string
}

export interface MinimapProps {
  /** The root box's caption — the project. */
  rootLabel: string
  /** Every lane to draw; see `lib/initiative-map.lanesOf`. */
  lanes: LaneWithEpics[]
  /** Select an initiative as the document being read, on this page. */
  onOpenInitiative: (id: string) => void
  /** Where an epic opens. */
  epicHref: (id: string) => string
  /** Where the verification surface lives. */
  verificationHref: string
  /** False while the roadmap has no project to be about. */
  hasProject: boolean
  labels: MinimapLabels
  className?: string
}

const MIN_ZOOM = 0.4
const MAX_ZOOM = 1.4
const ZOOM_STEP = 0.2

export function Minimap({
  rootLabel,
  lanes,
  onOpenInitiative,
  epicHref,
  verificationHref,
  hasProject,
  labels,
  className,
}: MinimapProps) {
  const [zoom, setZoom] = useState(1)
  const [epicKey, setEpicKey] = useState<string | null>(null)
  const [menuKey, setMenuKey] = useState<string | null>(null)

  const map = useMemo(() => layoutMap(rootLabel, lanes), [rootLabel, lanes])
  const nodes = useMemo(() => byKey(map.nodes), [map])

  // A selection outliving the node it named is the one way this can render a
  // panel about work that is no longer there — a refresh mid-read is enough.
  useEffect(() => {
    if (epicKey && !nodes.has(epicKey)) setEpicKey(null)
    if (menuKey && !nodes.has(menuKey)) setMenuKey(null)
  }, [nodes, epicKey, menuKey])

  const selectedEpic = useMemo(() => {
    const n = epicKey ? nodes.get(epicKey) : undefined
    return n?.kind === 'epic' ? n.epic : null
  }, [nodes, epicKey])

  // Escape closes the menu wherever focus is: a dropdown that can only be
  // dismissed by finding it again is a trap on a map you have to scroll.
  useEffect(() => {
    if (!menuKey) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMenuKey(null)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [menuKey])

  const nudgeZoom = useCallback((by: number) => {
    setZoom((z) => Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.round((z + by) * 10) / 10)))
  }, [])

  if (!hasProject) {
    return (
      <div className={cn('flex min-h-0 items-center justify-center px-6 py-10', className)}>
        <div className="max-w-md text-center">
          <p className="text-foreground m-0 text-[15px] font-semibold">{labels.needProject}</p>
          <p className="text-muted-foreground mt-1 mb-0 text-[13px]">{labels.needProjectHint}</p>
        </div>
      </div>
    )
  }

  return (
    <div className={cn('grid min-h-0 grid-cols-1 md:grid-cols-[1fr_320px]', className)}>
      {/* --- the map ------------------------------------------------------ */}
      <section className="flex min-h-0 min-w-0 flex-col">
        <header className="border-b-border-soft flex flex-none flex-wrap items-center gap-2 border-b px-3.5 py-2">
          <span className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
            {labels.heading}
          </span>
          <span className="text-muted-foreground flex items-center gap-1.5 text-[11px]">
            <span className="bg-okbg border-okbd inline-block size-2.5 rounded-[3px] border" />
            {labels.complete}
          </span>
          <span className="grow" />
          <div className="flex items-center gap-1">
            <Hint text={labels.zoomOut}>
              <Button
                variant="outline"
                size="icon"
                className="size-7"
                aria-label={labels.zoomOut}
                disabled={zoom <= MIN_ZOOM}
                onClick={() => nudgeZoom(-ZOOM_STEP)}
              >
                −
              </Button>
            </Hint>
            <span className="text-muted-foreground w-10 text-center font-mono text-[11px] tabular-nums">
              {Math.round(zoom * 100)}%
            </span>
            <Hint text={labels.zoomIn}>
              <Button
                variant="outline"
                size="icon"
                className="size-7"
                aria-label={labels.zoomIn}
                disabled={zoom >= MAX_ZOOM}
                onClick={() => nudgeZoom(ZOOM_STEP)}
              >
                +
              </Button>
            </Hint>
          </div>
        </header>

        {lanes.length === 0 ? (
          <div className="px-6 py-10 text-center">
            <p className="text-foreground m-0 text-[15px] font-semibold">{labels.empty}</p>
            <p className="text-muted-foreground mt-1 mb-0 text-[13px]">{labels.emptyHint}</p>
          </div>
        ) : (
          <div
            className="min-h-0 grow overflow-auto"
            // A click on the background dismisses the menu. On the scroller
            // rather than on a full-size overlay, so it never sits between the
            // pointer and a node.
            onClick={(e) => {
              if (e.target === e.currentTarget) setMenuKey(null)
            }}
          >
            <div
              style={{
                width: map.width * zoom,
                height: map.height * zoom,
                position: 'relative',
              }}
            >
              <div
                style={{
                  width: map.width,
                  height: map.height,
                  transform: `scale(${zoom})`,
                  transformOrigin: '0 0',
                  position: 'absolute',
                  inset: 0,
                }}
              >
                <svg
                  width={map.width}
                  height={map.height}
                  aria-hidden="true"
                  className="pointer-events-none absolute inset-0"
                >
                  {map.edges.map((e) => {
                    const from = nodes.get(e.from)
                    const to = nodes.get(e.to)
                    if (!from || !to) return null
                    return (
                      <path
                        key={e.from + '>' + e.to}
                        d={edgePath(from, to)}
                        fill="none"
                        className="stroke-border"
                        strokeWidth={1.5}
                      />
                    )
                  })}
                </svg>

                {map.nodes.map((n) => (
                  <NodeBox
                    key={n.key}
                    node={n}
                    selected={n.key === epicKey}
                    menuOpen={n.key === menuKey}
                    labels={labels}
                    onEpic={() => {
                      setMenuKey(null)
                      setEpicKey((k) => (k === n.key ? null : n.key))
                    }}
                    onLane={() => {
                      setEpicKey(null)
                      setMenuKey((k) => (k === n.key ? null : n.key))
                    }}
                  />
                ))}

                {/* The go-to menu, drawn in map space under the node it belongs
                    to: it scrolls with that node, so no scroll offset has to be
                    tracked to keep the two together. */}
                {map.nodes.map((n) =>
                  n.kind === 'lane' && n.key === menuKey ? (
                    <GoToMenu
                      key={'m:' + n.key}
                      node={n}
                      epicHref={epicHref}
                      verificationHref={verificationHref}
                      labels={labels}
                      onOpenDocument={() => {
                        setMenuKey(null)
                        onOpenInitiative(n.lane.id)
                      }}
                      onDismiss={() => setMenuKey(null)}
                    />
                  ) : null,
                )}
              </div>
            </div>
          </div>
        )}
      </section>

      {/* --- an epic's numbers -------------------------------------------- */}
      <aside className="border-t-border-soft md:border-l-border-soft bg-card min-h-0 overflow-y-auto border-t md:border-t-0 md:border-l">
        {selectedEpic ? (
          <EpicStats
            epic={selectedEpic}
            href={epicHref(selectedEpic.id)}
            labels={labels}
            onClose={() => setEpicKey(null)}
          />
        ) : (
          <div className="px-4 py-6">
            <p className="text-muted-foreground m-0 text-[12.5px]">{labels.pickEpic}</p>
          </div>
        )}
      </aside>
    </div>
  )
}

/** One box. Absolutely placed from the layout, which owns every coordinate. */
function NodeBox({
  node,
  selected,
  menuOpen,
  labels,
  onEpic,
  onLane,
}: {
  node: MapNode
  selected: boolean
  menuOpen: boolean
  labels: MinimapLabels
  onEpic: () => void
  onLane: () => void
}) {
  const box = {
    position: 'absolute' as const,
    left: node.x,
    top: node.y,
    width: node.width,
    height: node.height,
  }

  if (node.kind === 'root') {
    return (
      <div
        style={box}
        className="bg-secondary text-secondary-foreground border-border flex items-center rounded-[9px] border px-3"
      >
        <span className="truncate text-[13.5px] font-[740]">{node.label}</span>
      </div>
    )
  }

  if (node.kind === 'lane') {
    const { lane, unfiled } = node
    return (
      <button
        type="button"
        style={box}
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        aria-label={`${lane.title || labels.unfiled} — ${labels.goTo}`}
        onClick={onLane}
        className={cn(
          'bg-card hover:border-ring flex cursor-pointer flex-col justify-center gap-1 rounded-[9px] border px-3 text-left',
          menuOpen ? 'border-ring' : 'border-border',
          unfiled && 'border-dashed',
        )}
      >
        <span className="flex items-center gap-1.5">
          <span
            className={cn(
              'min-w-0 truncate text-[13px] font-[680]',
              unfiled && 'text-muted-foreground italic',
            )}
          >
            {unfiled ? labels.unfiled : lane.title}
          </span>
          <span className="text-muted-foreground ml-auto shrink-0 font-mono text-[10.5px] tabular-nums">
            {lane.done}/{lane.total}
          </span>
        </span>
        <Bar percent={lane.percent} complete={lane.total > 0 && lane.done >= lane.total} />
      </button>
    )
  }

  const { epic } = node
  const complete = isComplete(epic)
  return (
    <button
      type="button"
      style={box}
      aria-pressed={selected}
      onClick={onEpic}
      className={cn(
        'flex cursor-pointer flex-col justify-center gap-1 rounded-[9px] border px-3 text-left',
        // The one colour decision. Everything not finished stays neutral, so
        // green means "nothing left here" and not "this one is special".
        complete ? 'bg-okbg border-okbd' : 'bg-card border-border',
        selected ? 'border-ring ring-ring/40 ring-2' : 'hover:border-ring',
      )}
    >
      <span className="flex items-center gap-1.5">
        <span className="min-w-0 truncate text-[12.5px] font-[620]">{epic.title}</span>
        <span className="text-muted-foreground ml-auto shrink-0 font-mono text-[10.5px] tabular-nums">
          {epic.done}/{epic.total}
        </span>
      </span>
      <Bar percent={epic.percent} complete={complete} />
    </button>
  )
}

/** A 0-100 bar, the same vocabulary the versions strip and the CLI use. */
function Bar({ percent, complete }: { percent: number; complete: boolean }) {
  const pct = Math.max(0, Math.min(100, percent))
  return (
    <span
      className="bg-secondary block h-1 w-full overflow-hidden rounded-full"
      role="progressbar"
      aria-valuenow={pct}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <span
        className={cn('block h-full rounded-full', complete ? 'bg-ok' : 'bg-accent')}
        style={{ width: `${pct}%` }}
      />
    </span>
  )
}

/**
 * Where an initiative can take you.
 *
 * Its versions are listed as entries rather than left to be found on the map,
 * because the reason to open this menu on a lane with eleven of them is usually
 * to get to one — and every entry here is a link that resolves, which is the
 * bar for putting a destination in a menu at all.
 */
function GoToMenu({
  node,
  epicHref,
  verificationHref,
  labels,
  onOpenDocument,
  onDismiss,
}: {
  node: Extract<MapNode, { kind: 'lane' }>
  epicHref: (id: string) => string
  verificationHref: string
  labels: MinimapLabels
  onOpenDocument: () => void
  onDismiss: () => void
}) {
  const ref = useRef<HTMLDivElement>(null)
  // Held in a ref so the effect below runs ONCE. With the callback in the
  // dependency list an inline arrow at the call site re-runs it every render,
  // which re-arms the timeout each time and leaves the listener attached only
  // because renders happen to stop.
  const dismiss = useRef(onDismiss)
  dismiss.current = onDismiss

  // Dismiss on a press anywhere outside. `mousedown` rather than `click` so the
  // menu is gone before the thing underneath reacts.
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) dismiss.current()
    }
    // Deferred a tick: the click that opened the menu is still propagating.
    const id = window.setTimeout(() => document.addEventListener('mousedown', onDown), 0)
    return () => {
      window.clearTimeout(id)
      document.removeEventListener('mousedown', onDown)
    }
  }, [])

  const versions = node.lane.epics
  const entry =
    'block w-full cursor-pointer truncate px-3 py-1.5 text-left text-[12.5px] hover:bg-muted'

  return (
    <div
      ref={ref}
      role="menu"
      aria-label={labels.goTo}
      style={{
        position: 'absolute',
        left: node.x + 12,
        top: node.y + NODE_H + 6,
        width: node.width,
        zIndex: 20,
      }}
      className="bg-popover border-border overflow-hidden rounded-[9px] border py-1 shadow-lg"
    >
      {/* The unfiled bucket is not an initiative anybody created, so it has no
          document to open — only its versions are real destinations. */}
      {!node.unfiled && (
        <button type="button" role="menuitem" className={entry} onClick={onOpenDocument}>
          {labels.openDocument}
        </button>
      )}
      {!node.unfiled && (
        <a role="menuitem" className={entry} href={verificationHref} onClick={onDismiss}>
          {labels.openVerification}
        </a>
      )}
      {versions.length > 0 && (
        <>
          <div className="border-t-border-soft text-muted-foreground mt-1 border-t px-3 pt-1.5 pb-0.5 text-[10px] font-bold tracking-[0.05em] uppercase">
            {labels.versions}
          </div>
          {versions.map((id) => (
            <a key={id} role="menuitem" className={entry} href={epicHref(id)} onClick={onDismiss}>
              {id}
            </a>
          ))}
        </>
      )}
      {node.unfiled && versions.length === 0 && (
        <div className="text-muted-foreground px-3 py-1.5 text-[12px]">{labels.unfiledHint}</div>
      )}
    </div>
  )
}

/**
 * One epic's numbers.
 *
 * `by_state` is shown rather than only the totals because "eleven left" and
 * "eleven left, nine of them blocked" are different situations, and the totals
 * cannot tell them apart. `awaiting` reads as a warning rather than a bucket:
 * it is an overlay on the others, so it must not look like it adds up with them.
 */
function EpicStats({
  epic,
  href,
  labels,
  onClose,
}: {
  epic: RoadmapEpic
  href: string
  labels: MinimapLabels
  onClose: () => void
}) {
  const states = Object.entries(epic.by_state ?? {}).filter(([, n]) => n > 0)
  return (
    <div className="px-4 py-3.5">
      <div className="flex items-start gap-2">
        <div className="min-w-0 grow">
          <div className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
            {labels.statistics}
          </div>
          <h2 className="mt-1 mb-0 text-[15px] leading-snug font-[720]">{epic.title}</h2>
          <div className="text-muted-foreground mt-0.5 font-mono text-[11px]">{epic.id}</div>
        </div>
        <Hint text={labels.close}>
          <Button
            variant="outline"
            size="icon"
            className="size-7 shrink-0"
            aria-label={labels.close}
            onClick={onClose}
          >
            ×
          </Button>
        </Hint>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-1.5">
        <Badge
          variant="secondary"
          className="rounded-[5px] px-1.75 py-0.5 text-[10.5px] font-[750] tracking-[0.04em] uppercase"
        >
          {epic.state}
        </Badge>
        <span className="text-muted-foreground text-[11.5px]">
          {labels.priority} {epic.priority}
        </span>
      </div>

      <div className="mt-3">
        <div className="flex items-baseline gap-2">
          <span className="text-[19px] font-[740] tabular-nums">{epic.percent}%</span>
          <span className="text-muted-foreground text-[12px] tabular-nums">
            {epic.done} / {epic.total} {labels.done}
          </span>
        </div>
        <div className="mt-1.5">
          <Bar percent={epic.percent} complete={isComplete(epic)} />
        </div>
      </div>

      {epic.total === 0 ? (
        <p className="text-muted-foreground mt-3 mb-0 text-[12.5px]">{labels.noWork}</p>
      ) : (
        <dl className="mt-3 mb-0 grid grid-cols-2 gap-x-3 gap-y-1.5 text-[12.5px]">
          <Stat label={labels.ready} value={epic.ready} />
          <Stat label={labels.backlog} value={epic.backlog} />
          <Stat label={labels.awaiting} value={epic.awaiting_answer} warn />
        </dl>
      )}

      {states.length > 0 && (
        <div className="mt-3.5">
          <div className="text-muted-foreground text-[10.5px] font-bold tracking-[0.05em] uppercase">
            {labels.state}
          </div>
          <ul className="mt-1.5 mb-0 list-none space-y-1 p-0">
            {states.map(([state, n]) => (
              <li key={state} className="flex items-baseline gap-2 text-[12.5px]">
                <span className="min-w-0 truncate">{state}</span>
                <span className="border-b-border-soft grow border-b border-dotted" />
                <span className="shrink-0 tabular-nums">{n}</span>
              </li>
            ))}
          </ul>
        </div>
      )}

      <a
        href={href}
        className="text-accent mt-4 inline-block text-[12.5px] font-semibold hover:underline"
      >
        {labels.openOnBoard} →
      </a>
    </div>
  )
}

function Stat({ label, value, warn }: { label: string; value: number; warn?: boolean }) {
  return (
    <>
      <dt className="text-muted-foreground">{label}</dt>
      <dd
        className={cn(
          'm-0 text-right tabular-nums',
          warn && value > 0 && 'text-[color:var(--warn,#c99a3a)] font-semibold',
        )}
      >
        {value}
      </dd>
    </>
  )
}
