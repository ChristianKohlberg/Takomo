import { documentAppearanceStyle, type DocumentAppearance } from '@/lib/document-appearance'
import { usePersonalSelection } from '@/hooks/usePersonalSelection'
import { type SyncConnection } from '@/hooks/useSyncConnection'
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
// **Proposals belong to a SECTION.** They live in the same document, in the
// top-level `proposals` map, each carrying the node it is about — so an agent's
// offer arrives in an open browser at once, and the section it is about is the
// one that says something is waiting. Accepting one applies its ops to that
// section's fragment THROUGH ITS EDITOR, because markdown→ProseMirror needs the
// editor's exact schema and only the editor has it; the server deliberately does
// not construct nodes (`src/api/docprops.rs`). A decision is recorded on the
// proposal, never erased.
//
// Titles and new sections are edited inline against that same map tree.
import { InlineSection } from '@/components/documents/InlineSection'
import { insertPlanSection } from '@/lib/plan-insert'
import type { Locale } from '@/lib/i18n'
import { ChevronDownIcon } from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import * as Y from 'yjs'

import type { Editor } from '@tiptap/react'

import { OutlineRail, type OutlineRailLabels } from '@/components/documents/OutlineRail'
import { ProposalPanel, type ProposalPanelLabels } from '@/components/documents/ProposalPanel'
import { SectionPanel, type SectionPanelLabels } from '@/components/documents/SectionPanel'
import { applyOps, blockText, type Proposal } from '@/lib/doc-ops'
import {
  nodesMap,
  proseOf,
  proseTextOf,
  readPlanTree,
  readProseOf,
  setTitle,
} from '@/lib/mindmap-crdt'
import type { PlanStanding, TraceEntry } from '@/lib/mindmaps'
import { type MindmapSession } from '@/lib/mindmaps'
import {
  decideProposal,
  highlightKeyFor,
  pendingByNode,
  PROPOSALS_KEY,
  proposalsByNode,
  readProposals,
} from '@/lib/plan-proposals'
import {
  ancestorKeys,
  flattenSections,
  planSections,
  sameTree,
  visibleSections,
  type PlanNode,
} from '@/lib/plan-sections'
import { standingOf, type Standing } from '@/lib/plan-trace'
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
  appearance?: DocumentAppearance
  conversationFor?: (node: string) => ReactNode
  locale?: Locale
  testsFor: (node: string) => { total: number; failing: number }
  onShowTests: (node: string) => void
  testsLabel: string
  failedLabel: string
  onError: (error: unknown) => void
  session: MindmapSession
  standing: PlanStanding
  /** The plan's history, newest first, already split by section. */
  trace: ReadonlyMap<string, TraceEntry[]>
  connection: SyncConnection
  /** Somebody says they have read this section. */
  onReview: (node: string) => void
  /** A local edit settled. Debounced in the editor — see `SectionEditor`. */
  onEdited: (node: string) => void
  onShowOnMap: (node: string) => void
  /** A decision on a proposal, for the plan's history. */
  onDecided: (node: string, kind: 'accepted' | 'rejected') => void
  /** Ops that could not be applied after all, so they are reported rather than
   *  silently dropped. */
  onSkipped: (messages: string[]) => void
  /** A section handed over by link (`/documents#n=`), or null. Honoured once. */
  focusSection?: string | null
  onSelection?: (node: string | null) => void
  labels: PlanLabels
  railLabels: OutlineRailLabels
  sectionLabels: SectionPanelLabels
  proposalLabels: ProposalPanelLabels
}

export default function Plan(props: PlanProps) {
  return <ConnectedPlan {...props} />
}

function ConnectedPlan({
  appearance,
  conversationFor,
  locale = 'en',
  connection,
  testsFor,
  onShowTests,
  testsLabel,
  failedLabel,
  session,
  standing,
  trace,
  onReview,
  onEdited,
  onShowOnMap,
  onDecided,
  onSkipped,
  focusSection = null,
  onSelection,
  labels,
  railLabels,
  sectionLabels,
  proposalLabels,
}: PlanProps & { connection: SyncConnection }) {
  const { ydoc, provider } = connection

  const [tree, setTree] = useState<PlanNode[]>([])
  const [collapsed, setCollapsed] = useState<Set<string>>(() => loadFold(session.mindmap))
  const [selected, setSelected] = usePersonalSelection(onSelection)
  const focused = useRef<string | null>(null)

  const [openHistory, setOpenHistory] = useState<Set<string>>(() => new Set())
  const [openProposals, setOpenProposals] = useState<Set<string>>(() => new Set())
  const [proposals, setProposals] = useState<Proposal[]>([])
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

  // Proposals live BESIDE the prose in the same document, which is what makes
  // one appear in an open browser the moment an agent writes it and what keeps
  // it there across a disconnect. A proposal parked server-side until somebody
  // reloaded would be a second source of truth about the same plan.
  const proposalMap = useMemo(() => ydoc.getMap<string>(PROPOSALS_KEY), [ydoc])

  useEffect(() => {
    const read = () => setProposals(readProposals(proposalMap))
    read()
    proposalMap.observe(read)
    return () => proposalMap.unobserve(read)
  }, [proposalMap])

  const sections = useMemo(() => planSections(tree), [tree])
  const rows = useMemo(() => flattenSections(sections), [sections])

  const standings = useMemo(() => {
    const out: Record<string, Standing> = {}
    for (const row of rows) out[row.key] = standingOf(standing[row.key])
    return out
  }, [rows, standing])

  const proposalsFor = useMemo(() => proposalsByNode(proposals), [proposals])
  const pending = useMemo(() => pendingByNode(proposals), [proposals])
  /** The blocks each section's pending proposals are about, as a value the
   *  editor's effect can compare. See `highlightKeyFor`. */
  const highlights = useMemo(() => {
    const out: Record<string, string> = {}
    for (const [node, list] of proposalsFor) out[node] = highlightKeyFor(list)
    return out
  }, [proposalsFor])

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
    [sections, setSelected],
  )

  const scrolledSelection = useRef<string | null>(null)
  useEffect(() => {
    if (selected === scrolledSelection.current) return
    scrolledSelection.current = selected
    if (!selected) return
    const el = elements.current.get(selected)
    if (el) el.scrollIntoView({ block: 'start', behavior: 'smooth' })
  }, [selected])

  // A section handed over by link. Waits for the section to exist: the ask
  // arrives with the URL and the document is still syncing.
  useEffect(() => {
    if (!focusSection) {
      focused.current = null
      setSelected(null)
      return
    }
    if (focused.current === focusSection) return
    if (!rows.some((row) => row.key === focusSection)) return
    onSelect(focusSection)
    focused.current = focusSection
  }, [focusSection, rows, onSelect, setSelected])

  const onToggleHistory = useCallback((key: string) => {
    setOpenHistory((current) => {
      const next = new Set(current)
      if (!next.delete(key)) next.add(key)
      return next
    })
  }, [])

  const onToggleProposals = useCallback((key: string) => {
    setOpenProposals((current) => {
      const next = new Set(current)
      if (!next.delete(key)) next.add(key)
      return next
    })
  }, [])

  const visible = useMemo(() => visibleSections(sections, collapsed), [sections, collapsed])

  // ---- deciding on a proposal ---------------------------------------------

  /**
   * The mounted editors, by section.
   *
   * Accepting means applying ops to a real ProseMirror document, and only the
   * editor knows the schema those ops have to become nodes in. A section that is
   * not mounted has no editor and therefore no decision to make — which is
   * consistent, because its panel is not on screen either.
   *
   * Memoised per section for the same reason `refFor` is: an inline arrow would
   * re-register every editor on every render.
   */
  const pendingEditorFocus = useRef<string | null>(null)
  const insertSection = (after: string | null, level: 1 | 2 | 3, title: string): boolean => {
    if (!canWrite) return false
    const key = insertPlanSection(ydoc, after, level, title, session.display)
    if (!key) return false
    pendingEditorFocus.current = key
    setCollapsed(new Set())
    setNear((current) => current === null ? null : new Set([...current, key]))
    setSelected(key)
    return true
  }

  const editors = useRef(new Map<string, Editor>())
  const editorRefs = useRef(new Map<string, (editor: Editor | null) => void>())
  const editorRefFor = useCallback((key: string) => {
    const existing = editorRefs.current.get(key)
    if (existing) return existing
    const fn = (editor: Editor | null) => {
      if (editor) {
        editors.current.set(key, editor)
        if (pendingEditorFocus.current === key) {
          pendingEditorFocus.current = null
          editor.commands.focus('start')
        }
      }
      else editors.current.delete(key)
    }
    editorRefs.current.set(key, fn)
    return fn
  }, [])

  /** The current text of a block, for the before-side of a diff. */
  const textForIn = useCallback(
    (key: string) => (id: string) => {
      const editor = editors.current.get(key)
      return editor ? blockText(editor.state.doc, id) : null
    },
    [],
  )

  const onAccept = useCallback(
    (key: string, p: Proposal) => {
      const editor = editors.current.get(key)
      if (!editor) return
      const tr = editor.state.tr
      const { applied, skipped } = applyOps(tr, editor.schema, p.ops)
      // Nothing applied is not an acceptance. Recording one would leave a
      // durable claim that somebody accepted a change the document never
      // received — the reviewer saw only a toast, and after a reload there was
      // no trace at all. Say so and leave it pending, so it can be re-read
      // against the document as it now stands.
      if (!applied) {
        onSkipped(skipped.length ? skipped : [p.id])
        return
      }
      // Checked before dispatching, which catches a double click and a peer that
      // has already seen another reviewer's decision. It is not consensus — see
      // `decideProposal`.
      if (!decideProposal(proposalMap, p.id, 'accepted', session.display, Date.now(), skipped))
        return
      editor.view.dispatch(tr)
      // An op whose block has gone since the proposal was made is dropped —
      // reported rather than silently, because a reviewer who thinks they
      // accepted the whole change has not reviewed it. It is also written onto
      // the record above, so the difference survives a reload.
      if (skipped.length) onSkipped(skipped)
      onDecided(key, 'accepted')
    },
    [proposalMap, session.display, onSkipped, onDecided],
  )

  // Renaming a section from the plan.
  //
  // `setTitle` applies a DIFF to the title's `Y.Text`, which is what the map's
  // own rename does — so two people renaming one section merge rather than one
  // clobbering the other. It is only safe because `EditableText` commits on
  // blur: a live caret here would be a second one on the same text.
  const onTitle = useCallback(
    (key: string, text: string) => {
      if (!canWrite) return
      setTitle(ydoc, key, text)
      onEdited(key)
    },
    [canWrite, ydoc, onEdited],
  )

  const onReject = useCallback(
    (key: string, p: Proposal) => {
      // Nothing is applied and nothing is removed: the record stays, as
      // rejected, because it is a signal about the plan somebody was wrong about.
      if (!decideProposal(proposalMap, p.id, 'rejected', session.display)) return
      onDecided(key, 'rejected')
    },
    [proposalMap, session.display, onDecided],
  )

  /**
   * The prose fragment each mounted section is bound to.
   *
   * Resolved in an effect rather than while rendering, because resolving it can
   * WRITE: a node that predates prose has no fragment, and making one is a
   * change to the shared document. A render that React discards must not leave a
   * write behind, and a reader must not change the plan by looking at it — hence
   * the read-only path for a session that cannot write.
   */
  // Remembered per browser, not per session: somebody who folds the outline away
  // wants it folded next time too, and this is a per-viewer preference that
  // never needs to reach the server or another peer.
  const [outlineOpen, setOutlineOpen] = useState(() => {
    try {
      return localStorage.getItem('takomo.plan.outline') !== 'closed'
    } catch {
      return true
    }
  })
  useEffect(() => {
    try {
      localStorage.setItem('takomo.plan.outline', outlineOpen ? 'open' : 'closed')
    } catch {
      // A private window refuses storage; the fold still works for this visit.
    }
  }, [outlineOpen])

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
      {/* Collapsible, and the state is remembered.
          On a long plan the outline is how you navigate; on a narrow window it
          is competing with the prose for the only column that matters. Both are
          true at different moments, so it folds to a strip you can open again
          rather than a choice made once in the layout. */}
      <aside
        className={[
          'border-b-border-soft flex flex-none flex-col border-b bg-white md:border-r md:border-b-0 dark:bg-card',
          outlineOpen
            ? 'max-h-[38vh] overflow-y-auto px-2 py-3 md:max-h-none md:w-full md:max-w-80'
            : 'px-2 py-2 md:w-auto',
        ].join(' ')}
      >
        <button
          type="button"
          onClick={() => setOutlineOpen((v) => !v)}
          aria-expanded={outlineOpen}
          className="text-muted-foreground hover:text-foreground mb-1 flex items-center gap-1.5 self-start rounded-md px-1.5 py-1 text-[12px] font-[650]"
        >
          <ChevronDownIcon
            className={[
              'size-3.5 flex-none transition-transform',
              outlineOpen ? '' : '-rotate-90',
            ].join(' ')}
            aria-hidden="true"
          />
          <span>{railLabels.outline}</span>
        </button>
        {outlineOpen && (
          <OutlineRail
            sections={sections}
            selected={selected}
            onSelect={onSelect}
            collapsed={collapsed}
            onToggle={onToggleFold}
            standing={standings}
            pending={pending}
            labels={railLabels}
          />
        )}
      </aside>

      {/* The document's own ground is white — the plan is the one surface here
          meant to read like a page rather than like an app, so it does not take
          the muted app background. In dark mode it stays on the card colour
          rather than becoming a glaring white rectangle. */}
      <div
        ref={columnRef}
        className="min-w-0 flex-1 overflow-y-auto bg-white px-4 py-2 md:px-6 dark:bg-card"
      >
        {!canWrite && (
          <p className="mb-3 rounded-md border border-amber-300 bg-amber-50 px-3 py-2 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-200">
            {labels.readOnly}
          </p>
        )}

        {rows.length === 0 ? (
          <div className="text-muted-foreground py-8 text-[13.5px]">
            {canWrite ? (
              <InlineSection locale={locale} maxLevel={1} onInsert={(level, title) => insertSection(null, level, title)} />
            ) : <p>{labels.empty}</p>}
          </div>
        ) : (
          // A measure, not a width: prose running the full width of a desktop
          // window is unreadable, and `max-w-*` cannot overflow a phone.
          // `mx-auto` centres that measure in whatever column is left once the
          // outline has taken its share.
          <div className="document-appearance mx-auto min-w-0 max-w-[720px]"
            style={documentAppearanceStyle(appearance)}>
            {canWrite && <InlineSection locale={locale} maxLevel={1} onInsert={(level, title) => insertSection(null, level, title)} />}
            {visible.map((row) => {
              const mounted = near === null || near.has(row.key)
              const fragment = mounted ? (fragments.get(row.key) ?? null) : null
              const preview = previews.get(row.key) ?? ''
              const offered = proposalsFor.get(row.key) ?? []
              return (
                <SectionPanel
                  key={row.key}
                  number={row.number}
                  depth={row.depth}
                  title={row.title}
                  onTitle={(text) => onTitle(row.key, text)}
                  onHeadingEnter={() => editors.current.get(row.key)?.commands.focus('start')}
                  standing={standings[row.key] ?? 'unseen'}
                  entries={trace.get(row.key) ?? []}
                  historyOpen={openHistory.has(row.key)}
                  onToggleHistory={() => onToggleHistory(row.key)}
                  onReview={() => onReview(row.key)}
                  onShowOnMap={() => onShowOnMap(row.key)}
                  onShowTests={() => onShowTests(row.key)}
                  testsLabel={`${testsLabel} (${testsFor(row.key).total})${testsFor(row.key).failing ? ` · ${testsFor(row.key).failing} ${failedLabel}` : ''}`}
                  failingTests={testsFor(row.key).failing > 0}
                  pending={pending[row.key] ?? 0}
                  proposalCount={offered.length}
                  proposalsOpen={openProposals.has(row.key)}
                  onToggleProposals={() => onToggleProposals(row.key)}
                  proposals={
                    <ProposalPanel
                      proposals={offered}
                      textFor={textForIn(row.key)}
                      // Deciding writes the plan, so a reader gets the
                      // proposals and no buttons. An unmounted section has no
                      // editor to apply anything to either.
                      canWrite={canWrite && mounted}
                      onAccept={(p) => onAccept(row.key, p)}
                      onReject={(p) => onReject(row.key, p)}
                      labels={proposalLabels}
                    />
                  }
                  canWrite={canWrite}
                  active={selected === row.key}
                  onActivate={() => setSelected(row.key)}
                  sectionRef={refFor(row.key)}
                  labels={sectionLabels}
                >
                  {fragment ? (
                    <SectionEditor
                      locale={locale}
                      ydoc={ydoc}
                      fragment={fragment}
                      provider={provider}
                      display={session.display}
                      color={color}
                      canWrite={canWrite}
                      onInsertSection={(level, title) => insertSection(row.key, level, title)}
                      onSettled={() => onEdited(row.key)}
                      highlight={highlights[row.key] ?? ''}
                      onEditor={editorRefFor(row.key)}
                      label={labels.proseLabel.replace('{n}', row.number)}
                    />
                  ) : (
                    // Not a spinner: this is the section's own text, which is
                    // also what holds its height, so mounting an editor behind
                    // you does not move the page under your eyes.
                    <p className="document-prose text-muted-foreground min-h-[2.5rem] px-1 py-1 whitespace-pre-line">
                      {preview || labels.proseEmpty}
                    </p>
                  )}
                  {conversationFor?.(row.key)}
                  {canWrite && <InlineSection locale={locale} maxLevel={Math.min(row.depth + 2, 3)} onInsert={(level, title) => insertSection(row.key, level, title)} />}
                </SectionPanel>
              )
            })}
          </div>
        )}
      </div>
    </main>
  )
}
