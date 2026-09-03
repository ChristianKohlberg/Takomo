// The plan, written out: the map's tree read as reading order.
//
// This is `/mindmaps`' Live.tsx pointed at the same document from the other
// side. It opens the MAP's sync session — one socket for the whole view — and
// every section is an editor bound to that node's own `prose` fragment. There is
// no per-document session for plan content any more, because there is no
// per-section document: a node IS a section (`spec/one-model-two-views.md`).
//
// Three things are worth knowing before changing anything here.
//
// **Structure comes from a LIGHT read.** With prose inside the nodes, "the
// document changed" now includes every character anybody types, and the outline
// changes for none of them. `readPlanTree` reads the four fields the outline is
// made of, and `sameTree` keeps the projection when the shape did not move —
// without it a remote keystroke re-renders every section, and a section is a
// mounted editor.
//
// **Editors are mounted only near the viewport.** The cap is 500 sections and
// 500 ProseMirror instances is not a thing to do to a browser. An offscreen
// section shows its prose as plain text, which is also what holds its height, so
// scrolling does not jump as editors mount behind you.
//
// **Titles are not edited here.** A heading is read-only and the section offers
// "show it on the map" instead: the title caret lives on the canvas, and two
// carets on one Y.Text in two layouts is a fight rather than a feature.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { WebsocketProvider } from 'y-websocket'
import * as Y from 'yjs'

import { OutlineRail, type OutlineRailLabels } from '@/components/documents/OutlineRail'
import { SectionPanel, type SectionPanelLabels } from '@/components/documents/SectionPanel'
import type { TraceEntry } from '@/lib/mindmaps'
import { mindmapSyncBase, type MindmapSession } from '@/lib/mindmaps'
import {
  nodesMap,
  proseOf,
  proseTextOf,
  readPlanTree,
  readProseOf,
} from '@/lib/mindmap-crdt'
import {
  ancestorKeys,
  flattenSections,
  planSections,
  sameTree,
  visibleSections,
  type PlanNode,
} from '@/lib/plan-sections'
import { standingOf, type Standing } from '@/lib/plan-trace'
import type { PlanStanding } from '@/lib/mindmaps'
import SectionEditor from './SectionEditor'

/** Caret colours. Fixed palette, picked by hashing the name so it is stable —
 *  the same function the canvas uses, for the same reason. */
const CARET_COLORS = ['#2563eb', '#059669', '#d97706', '#dc2626', '#7c3aed', '#0891b2', '#c026d3']

function colorFor(name: string): string {
  let hash = 0
  for (let i = 0; i < name.length; i++) hash = (hash * 31 + name.charCodeAt(i)) | 0
  return CARET_COLORS[Math.abs(hash) % CARET_COLORS.length] ?? '#2563eb'
}

export type ConnectionState = 'connecting' | 'connected' | 'disconnected'

/** Fold is per-viewer: folding a branch of the plan must not fold it under
 *  somebody else who is reading it. Browser-local, keyed by map. */
const foldKey = (id: string) => `takomo.plan.fold.${id}`

function loadFold(id: string): Set<string> {
  try {
    const raw = localStorage.getItem(foldKey(id))
    const parsed: unknown = raw ? JSON.parse(raw) : []
    return new Set(Array.isArray(parsed) ? parsed.filter((x) => typeof x === 'string') : [])
  } catch {
    return new Set()
  }
}

/** How far outside the viewport a section still counts as worth mounting. One
 *  screen or so, so an editor is ready by the time it is read. */
const NEAR_MARGIN = '800px 0px'

export interface PlanLabels {
  readOnly: string
  empty: string
  emptyHint: string
  /** A section with nothing written in it yet. */
  proseEmpty: string
  /** The accessible name of a section's editor. `{n}` is its number. */
  proseLabel: string
}

export interface PlanProps {
  session: MindmapSession
  standing: PlanStanding
  /** The plan's history, newest first, already split by section. */
  trace: ReadonlyMap<string, TraceEntry[]>
  onConnection: (state: ConnectionState) => void
  onPeers: (names: string[]) => void
  /** Somebody says they have read this section. */
  onReview: (node: string) => void
  /** A local edit settled. Debounced in the editor — see `SectionEditor`. */
  onEdited: (node: string) => void
  onShowOnMap: (node: string) => void
  labels: PlanLabels
  railLabels: OutlineRailLabels
  sectionLabels: SectionPanelLabels
}

export default function Plan({
  session,
  standing,
  trace,
  onConnection,
  onPeers,
  onReview,
  onEdited,
  onShowOnMap,
  labels,
  railLabels,
  sectionLabels,
}: PlanProps) {
  // One Y.Doc and one provider per map, rebuilt only when the ticket changes.
  // Recreating either on an unrelated render would drop the connection and
  // resync from scratch under somebody's cursor.
  const { ydoc, provider } = useMemo(() => {
    const ydoc = new Y.Doc()
    // Base, room and params passed SEPARATELY: y-websocket composes
    // `serverUrl + "/" + room + "?" + params` itself, so a finished URL puts the
    // room after the query string and the socket goes somewhere unrouted.
    const provider = new WebsocketProvider(mindmapSyncBase(session), session.room, ydoc, {
      params: { ticket: session.token },
      connect: true,
    })
    return { ydoc, provider }
  }, [session])

  const [tree, setTree] = useState<PlanNode[]>([])
  const [collapsed, setCollapsed] = useState<Set<string>>(() => loadFold(session.mindmap))
  const [selected, setSelected] = useState<string | null>(null)
  const [openHistory, setOpenHistory] = useState<Set<string>>(() => new Set())
  const canWrite = session.can_write
  const color = useMemo(() => colorFor(session.display), [session.display])

  // The document is the source; React state is a projection of it. `observeDeep`
  // rather than `observe` because a title is a Y.Text inside a Y.Map inside a
  // Y.Map — and now a section's prose is a fragment in the same place, which is
  // exactly why the projection is only replaced when the SHAPE moved.
  useEffect(() => {
    const read = () => {
      const next = readPlanTree(ydoc)
      setTree((prev) => (sameTree(prev, next) ? prev : next))
    }
    read()
    const nm = nodesMap(ydoc)
    nm.observeDeep(read)
    return () => nm.unobserveDeep(read)
  }, [ydoc])

  useEffect(() => {
    const onStatus = ({ status }: { status: string }) => {
      onConnection(
        status === 'connected'
          ? 'connected'
          : status === 'connecting'
            ? 'connecting'
            : 'disconnected',
      )
    }
    const onSynced = (isSynced: boolean) => {
      if (isSynced) onConnection('connected')
    }
    // Seed from what the provider already IS: it connected during the `useMemo`
    // above, so a `status` event can land before this subscription does — which
    // is what once left a header reading "Connecting…" over a live document.
    if (provider.wsconnected) onConnection('connected')
    const onAwareness = () => {
      const names: string[] = []
      provider.awareness.getStates().forEach((state, clientId) => {
        if (clientId === provider.awareness.clientID) return
        const user = (state as { user?: { name?: string } }).user
        if (user?.name) names.push(user.name)
      })
      onPeers(names)
    }
    provider.on('status', onStatus)
    provider.on('sync', onSynced)
    provider.awareness.on('change', onAwareness)
    provider.awareness.setLocalStateField('user', { name: session.display, color })

    return () => {
      provider.off('status', onStatus)
      provider.off('sync', onSynced)
      provider.awareness.off('change', onAwareness)
      provider.destroy()
      ydoc.destroy()
    }
  }, [provider, ydoc, session.display, color, onConnection, onPeers])

  const sections = useMemo(() => planSections(tree), [tree])
  const rows = useMemo(() => flattenSections(sections), [sections])

  const standings = useMemo(() => {
    const out: Record<string, Standing> = {}
    for (const row of rows) out[row.key] = standingOf(standing[row.key])
    return out
  }, [rows, standing])

  /**
   * A line or two of each section, for the ones no editor is mounted on.
   *
   * Recomputed when the plan's SHAPE changes rather than on every keystroke: a
   * preview only matters for a section nobody is looking at, and walking five
   * hundred sections' text on every character typed anywhere is precisely the
   * cost this view is arranged to avoid.
   */
  const previews = useMemo(() => {
    const out = new Map<string, string>()
    for (const row of rows) out.set(row.key, proseTextOf(ydoc, row.key))
    return out
  }, [rows, ydoc])

  // ---- which sections are mounted -----------------------------------------

  const columnRef = useRef<HTMLDivElement | null>(null)
  const elements = useRef(new Map<string, HTMLElement>())
  const observer = useRef<IntersectionObserver | null>(null)
  // `null` means "mount everything", which is what a browser with no
  // IntersectionObserver — and jsdom — gets. Otherwise nothing is mounted until
  // the observer has said so, so opening a 500-section plan mounts a screenful.
  const [near, setNear] = useState<Set<string> | null>(() =>
    typeof IntersectionObserver === 'undefined' ? null : new Set(),
  )

  useEffect(() => {
    if (typeof IntersectionObserver === 'undefined') return
    const io = new IntersectionObserver(
      (batch) => {
        setNear((prev) => {
          const next = new Set(prev ?? [])
          let moved = false
          for (const entry of batch) {
            const id = (entry.target as HTMLElement).dataset.section
            if (!id) continue
            if (entry.isIntersecting) {
              if (!next.has(id)) {
                next.add(id)
                moved = true
              }
            } else if (next.delete(id)) {
              moved = true
            }
          }
          return moved ? next : prev
        })
      },
      { root: columnRef.current, rootMargin: NEAR_MARGIN },
    )
    observer.current = io
    for (const el of elements.current.values()) io.observe(el)
    return () => {
      io.disconnect()
      observer.current = null
    }
  }, [])

  /**
   * A stable ref callback per section.
   *
   * Memoised by id rather than made inline: an inline arrow is a new function
   * every render, so React would detach and re-attach every section's element on
   * each one — and each detach unobserves it, which is a scroll's worth of
   * churn for nothing.
   */
  const refs = useRef(new Map<string, (el: HTMLElement | null) => void>())
  const refFor = useCallback((id: string) => {
    const existing = refs.current.get(id)
    if (existing) return existing
    const fn = (el: HTMLElement | null) => {
      // The observer is handed elements, not ids, so the id rides on the element
      // it is about.
      if (el) el.dataset.section = id
      const previous = elements.current.get(id)
      if (previous && observer.current) observer.current.unobserve(previous)
      if (el) {
        elements.current.set(id, el)
        observer.current?.observe(el)
      } else {
        elements.current.delete(id)
      }
    }
    refs.current.set(id, fn)
    return fn
  }, [])

  // ---- fold, selection, and getting to a section ---------------------------

  useEffect(() => {
    try {
      localStorage.setItem(foldKey(session.mindmap), JSON.stringify([...collapsed]))
    } catch {
      // Private mode, or storage full. The fold still works for this visit.
    }
  }, [collapsed, session.mindmap])

  const onToggleFold = useCallback((key: string) => {
    setCollapsed((current) => {
      const next = new Set(current)
      if (!next.delete(key)) next.add(key)
      return next
    })
  }, [])

  const onSelect = useCallback(
    (key: string) => {
      // Unfold whatever hides it first: scrolling to a section that is not drawn
      // is worse than not scrolling.
      setCollapsed((current) => {
        const hiding = ancestorKeys(sections, key).filter((k) => current.has(k))
        if (hiding.length === 0) return current
        const next = new Set(current)
        for (const k of hiding) next.delete(k)
        return next
      })
      setSelected(key)
    },
    [sections],
  )

  useEffect(() => {
    if (!selected) return
    const el = elements.current.get(selected)
    if (el) el.scrollIntoView({ block: 'start', behavior: 'smooth' })
  }, [selected, rows])

  const onToggleHistory = useCallback((key: string) => {
    setOpenHistory((current) => {
      const next = new Set(current)
      if (!next.delete(key)) next.add(key)
      return next
    })
  }, [])

  const visible = useMemo(() => visibleSections(sections, collapsed), [sections, collapsed])

  /**
   * The prose fragment each mounted section is bound to.
   *
   * Resolved in an effect rather than while rendering, because resolving it can
   * WRITE: a node that predates prose has no fragment, and making one is a
   * change to the shared document. A render that React discards must not leave a
   * write behind, and a reader must not change the plan by looking at it — hence
   * the read-only path for a session that cannot write.
   */
  const [fragments, setFragments] = useState<Map<string, Y.XmlFragment>>(() => new Map())
  const known = useRef(fragments)
  useEffect(() => {
    const made = new Map<string, Y.XmlFragment>()
    for (const row of visible) {
      if (near !== null && !near.has(row.key)) continue
      if (known.current.has(row.key)) continue
      const frag = canWrite ? proseOf(ydoc, row.key) : readProseOf(ydoc, row.key)
      if (frag) made.set(row.key, frag)
    }
    if (made.size === 0) return
    const next = new Map(known.current)
    for (const [key, frag] of made) next.set(key, frag)
    known.current = next
    setFragments(next)
  }, [visible, near, canWrite, ydoc])

  return (
    <main className="flex min-h-0 flex-1 flex-col overflow-hidden md:flex-row">
      {/* One breakpoint, `md`, meaning phone or not: the outline stacks above
          the plan on a phone and sits beside it everywhere else. */}
      <aside className="border-b-border-soft flex max-h-[38vh] flex-none flex-col overflow-y-auto border-b px-2 py-3 md:max-h-none md:w-full md:max-w-80 md:border-r md:border-b-0">
        <OutlineRail
          sections={sections}
          selected={selected}
          onSelect={onSelect}
          collapsed={collapsed}
          onToggle={onToggleFold}
          standing={standings}
          labels={railLabels}
        />
      </aside>

      <div ref={columnRef} className="min-w-0 flex-1 overflow-y-auto px-4 py-2 md:px-6">
        {!canWrite && (
          <p className="mb-3 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-200">
            {labels.readOnly}
          </p>
        )}

        {rows.length === 0 ? (
          <div className="text-muted-foreground py-8 text-[13.5px]">
            <p>{labels.empty}</p>
            <p className="mt-2 opacity-80">{labels.emptyHint}</p>
          </div>
        ) : (
          // A measure, not a width: prose running the full width of a desktop
          // window is unreadable, and `max-w-*` cannot overflow a phone.
          <div className="min-w-0 max-w-[720px]">
            {visible.map((row) => {
              const mounted = near === null || near.has(row.key)
              const fragment = mounted ? (fragments.get(row.key) ?? null) : null
              const preview = previews.get(row.key) ?? ''
              return (
                <SectionPanel
                  key={row.key}
                  number={row.number}
                  depth={row.depth}
                  title={row.title}
                  standing={standings[row.key] ?? 'unseen'}
                  entries={trace.get(row.key) ?? []}
                  historyOpen={openHistory.has(row.key)}
                  onToggleHistory={() => onToggleHistory(row.key)}
                  onReview={() => onReview(row.key)}
                  onShowOnMap={() => onShowOnMap(row.key)}
                  canWrite={canWrite}
                  active={selected === row.key}
                  sectionRef={refFor(row.key)}
                  labels={sectionLabels}
                >
                  {fragment ? (
                    <SectionEditor
                      ydoc={ydoc}
                      fragment={fragment}
                      provider={provider}
                      display={session.display}
                      color={color}
                      canWrite={canWrite}
                      onSettled={() => onEdited(row.key)}
                      label={labels.proseLabel.replace('{n}', row.number)}
                    />
                  ) : (
                    // Not a spinner: this is the section's own text, which is
                    // also what holds its height, so mounting an editor behind
                    // you does not move the page under your eyes.
                    <p className="text-muted-foreground min-h-[2.5rem] px-1 py-1 text-[13.5px] whitespace-pre-line">
                      {preview || labels.proseEmpty}
                    </p>
                  )}
                </SectionPanel>
              )
            })}
          </div>
        )}
      </div>
    </main>
  )
}
