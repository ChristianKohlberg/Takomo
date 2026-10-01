import { SectionReferences } from '@/components/documents/SectionReferences'
import { DocumentReviewProvider } from '@/components/documents/DocumentReview'
import { highlightSearchPassage } from '@/lib/document-search-highlight'
import { useDocumentNumbering } from '@/components/documents/DocumentNumberingControls'
import { DocumentZoomControl, DocumentZoomSteps, useDocumentZoom } from '@/components/documents/DocumentZoom'
import { EmbeddingStatusProvider } from '@/hooks/useEmbeddingStatus'
import { DocumentEmbeddingStatus } from '@/components/documents/DocumentEmbeddingStatus'
import type { ServerSync } from '@/lib/save-status'
import { DocumentHybridSearch } from '@/components/documents/DocumentHybridSearch'
import { locatedPassageRange, type SearchResult } from '@/lib/hybrid-search'
import { DocumentSectionReferenceButton } from '@/components/documents/DocumentSectionReferenceButton'
import { DocumentActions } from '@/components/documents/DocumentActions'
import { CopySectionLink } from '@/components/documents/CopySectionLink'
import { DocumentComments } from '@/components/documents/DocumentComments'
import { DocumentCommentButton } from '@/components/documents/DocumentCommentButton'
import { resolveCommentAnchor, type CommentThread, type CommentAnchor } from '@/lib/document-comments'
import { specificationLink } from '@/lib/specification-url'
import { useDocumentSearch } from '@/hooks/useDocumentSearch'
import { highlightDocumentHeadings } from '@/lib/document-search-headings'
import { createStructureHistory, type SectionPlacement } from '@/lib/plan-structure'
import { MoveSectionDialog } from '@/components/documents/MoveSectionDialog'
import { documentAppearanceStyle, type DocumentAppearance } from '@/lib/document-appearance'
import { usePersonalSelection } from '@/hooks/usePersonalSelection'
import { type SyncConnection } from '@/hooks/useSyncConnection'
import type { FocusChange } from '@/hooks/useWorkspaceSection'
import { SectionFocusBand, SectionFocusBreadcrumb } from '@/components/documents/SectionFocus'
import { outsideFocus, sectionFocusScope } from '@/lib/section-focus'
import { readCommentThreads, COMMENT_FIELD } from '@/lib/document-comments'
import { isTextEntry } from '@/lib/mindmap-commands'
import { anchorScroll, cancelScrollAnchor, revealEditorPosition } from '@/lib/scroll-anchor'
import { SectionHeights } from '@/lib/section-heights'
import { pick } from '@/lib/i18n'
import { STR } from './strings'
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
// section shows its prose as plain text, held at the height it last had with an
// editor (`lib/section-heights.ts`), and going to a section anchors it while the
// editors around it mount (`lib/scroll-anchor.ts`).
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
import { DocumentStart } from './DocumentStart'
import { sectionSummaries, setSectionSummary, removeSectionSummary } from '@/lib/section-collapse'
import { insertPlanSection } from '@/lib/plan-insert'
import type { Locale } from '@/lib/i18n'
import { ChevronDownIcon, ChevronRight, ShieldAlert, ShieldCheck } from 'lucide-react'
import { cn } from '@/lib/utils'
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react'
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
  type PlanSection,
} from '@/lib/plan-sections'
import { standingOf, type Standing } from '@/lib/plan-trace'
import SectionEditor from './SectionEditor'

/** The pane width at which the outline stops being a drawer; the same 850 the stylesheet's container query uses. */
export const DOCUMENT_PANE_WIDE = 850

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
  traceState?: Record<string, import('@/hooks/useSectionTrace').SectionTraceState>
  onHistoryOpen?: (section: string, open: boolean) => void
  onHistoryRetry?: (section: string) => void
  token?: string
  userId?: string
  serverSync?: ServerSync
  agentTools?: ReactNode
  ticketLinksFor?: (section: string) => ReactNode
  project?: string
  focusMode?: boolean
  structureHistory?: ReturnType<typeof createStructureHistory> | null
  appearance?: DocumentAppearance
  conversationFor?: (node: string) => ReactNode
  locale?: Locale
  testsFor: (node: string) => { total: number; failing: number; verified?: number }
  onShowTests: (node: string) => void
  testsLabel: string
  failedLabel: string
  /** "verified", for the status line under a section that behaviors name. */
  verifiedLabel?: string
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
  onMoved?: (node: string) => void
  onShowOnMap: (node: string) => void
  /** A decision on a proposal, for the plan's history. */
  onDecided: (node: string, kind: 'accepted' | 'rejected') => void
  /** Ops that could not be applied after all, so they are reported rather than
   *  silently dropped. */
  onSkipped: (messages: string[]) => void
  /** A section handed over by link (`/documents#n=`), or null. Honoured once. */
  focusSection?: string | null
  onSelection?: (node: string | null) => void
  /** Section focus (`focus=`): the section whose subtree alone is rendered, or null. */
  sectionFocus?: string | null
  /** Enter, change or leave section focus. A history entry, never a document write. */
  onSectionFocus?: (node: string | null, change?: FocusChange) => void
  labels: PlanLabels
  railLabels: OutlineRailLabels
  sectionLabels: SectionPanelLabels
  proposalLabels: ProposalPanelLabels
}

export default function Plan(props: PlanProps) {
  const content = <ConnectedPlan {...props} />
  return props.token ? <EmbeddingStatusProvider key={`${props.session.mindmap}:${props.token}`} token={props.token} map={props.session.mindmap} server={props.serverSync}>{content}</EmbeddingStatusProvider> : content
}

function ConnectedPlan({
  traceState, onHistoryOpen, onHistoryRetry,
  token,
  project = '',
  userId,
  focusMode = false,
  structureHistory,
  appearance,
  conversationFor,
  agentTools,
  ticketLinksFor,
  locale = 'en',
  connection,
  testsFor,
  onShowTests,
  testsLabel,
  failedLabel,
  verifiedLabel = 'verified',
  session,
  standing,
  trace,
  onReview,
  onEdited,
  onMoved,
  onShowOnMap,
  onDecided,
  onSkipped,
  focusSection = null,
  onSelection,
  sectionFocus = null,
  onSectionFocus,
  labels,
  railLabels,
  sectionLabels,
  proposalLabels,
}: PlanProps & { connection: SyncConnection }) {
  const { ydoc, provider } = connection

  const [tree, setTree] = useState<PlanNode[]>([])
  const [treeRead, setTreeRead] = useState(false)
  const [summaries, setSummaries] = useState(() => sectionSummaries(ydoc))
  const collapsible = useMemo(() => new Set(Object.keys(summaries)), [summaries])
  const [collapsed, setCollapsed] = useState<Set<string>>(() => loadFold(session.mindmap))
  const [selected, setSelected, setSelectedQuietly] = usePersonalSelection(onSelection)
  const words = pick(STR, locale)
  const selectedRef = useRef(selected)
  selectedRef.current = selected
  const focused = useRef<string | null>(null)
  const numbering = useDocumentNumbering(project, appearance)
  const [moving, setMoving] = useState<string | null>(null)
  const [comments, setComments] = useState<{ section: string; draft: CommentAnchor | null } | null>(null)
  const [allComments, setAllComments] = useState(false)
  const pendingComment = useRef<CommentThread | null>(null)
  const [commentsEditor, setCommentsEditor] = useState<Editor | null>(null)
  const [notice, setNotice] = useState<{ text: string; undo?: boolean } | null>(null)
  const [history, setHistory] = useState<ReturnType<typeof createStructureHistory> | null>(null)
  const [, refreshMoveTools] = useState(0)
  useEffect(() => {
    const next = structureHistory ?? createStructureHistory(ydoc)
    setHistory(next)
    const unsubscribe = next.subscribe(() => refreshMoveTools(value => value + 1))
    return () => { unsubscribe(); if (!structureHistory) next.destroy() }
  }, [ydoc, structureHistory])
  useEffect(() => {
    if (!notice) return
    const timer = setTimeout(() => setNotice(null), 6000)
    return () => clearTimeout(timer)
  }, [notice])

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
      const nextSummaries = sectionSummaries(ydoc)
      setSummaries(prev => JSON.stringify(prev) === JSON.stringify(nextSummaries) ? prev : nextSummaries)
      const next = readPlanTree(ydoc)
      setTree((prev) => (sameTree(prev, next) ? prev : next))
      setTreeRead(true)
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

  // ---- section focus ---------------------------------------------------------
  // A personal VIEW narrowing: the focused section and its subtree are the only
  // rows handed to the renderer (and the outline), so no other section's editor
  // is even constructed. Numbers and depths are the real ones from `sections`.
  const scope = useMemo(() => sectionFocusScope(sections, sectionFocus), [sections, sectionFocus])
  const scopeRef = useRef(scope)
  scopeRef.current = scope
  const scopeSections = useMemo(() => (scope ? [scope.root] : sections), [scope, sections])
  const scopeRows = useMemo(() => (scope ? rows.filter(row => scope.ids.has(row.key)) : rows), [scope, rows])
  // A stale local replica must not drop a shared focus link before the server
  // has spoken; a provider without the flag (tests, offline stubs) counts as synced.
  const [synced, setSynced] = useState(() => (provider as { synced?: boolean }).synced !== false)
  useEffect(() => {
    if (synced) return
    const target = provider as unknown as { on?: (event: string, fn: (value: boolean) => void) => void; off?: (event: string, fn: (value: boolean) => void) => void }
    const onSync = (value: boolean) => { if (value) setSynced(true) }
    target.on?.('sync', onSync)
    return () => target.off?.('sync', onSync)
  }, [provider, synced])
  const searchNodes = useMemo(() => rows.map(row => ({ id: row.key, title: row.title })), [rows])
  const search = useDocumentSearch(ydoc, searchNodes)
  const activeMatch = search.activeMatch
  const activeSearchSection = activeMatch?.sectionId
  const activeSearchKey = activeMatch?.key
  const activeSearchKind = activeMatch?.kind
  const effectiveCollapsed = useMemo(() => {
    const revealed = new Set(activeSearchSection ? [activeSearchSection, ...ancestorKeys(sections, activeSearchSection)] : [])
    return new Set([...collapsed].filter(key => collapsible.has(key) && !revealed.has(key)))
  }, [collapsed, collapsible, sections, activeSearchSection])

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
    for (const row of scopeRows) out.set(row.key, proseTextOf(ydoc, row.key))
    return out
  }, [scopeRows, ydoc])

  // ---- which sections are mounted -----------------------------------------

  const columnRef = useRef<HTMLDivElement | null>(null)
  const zoom = useDocumentZoom(project, columnRef)
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

  /**
   * Put a section's top at the top of the column — at once, and kept there
   * while the editors around it mount and grow (`lib/scroll-anchor.ts`). The
   * focused section is the exception: it opens the column, so the column goes
   * to its very top and the breadcrumb above the section stays in view.
   * Nothing above that point can move, so there is nothing to anchor.
   */
  const revealSection = useCallback((key: string) => {
    const column = columnRef.current
    if (!column) return
    if (scopeRef.current?.root.key === key) {
      cancelScrollAnchor(column)
      column.scrollTop = 0
      return
    }
    anchorScroll({ container: column, target: () => elements.current.get(key) })
  }, [])
  /** A place inside a mounted section (a search hit): centred, then anchored. */
  const revealInColumn = useCallback((element: HTMLElement | null | undefined) => {
    if (element) anchorScroll({ container: columnRef.current, target: element, align: 'center' })
  }, [])

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
        const hiding = [key, ...ancestorKeys(sections, key)].filter((k) => current.has(k))
        if (hiding.length === 0) return current
        const next = new Set(current)
        for (const k of hiding) next.delete(k)
        return next
      })
      setSelected(key)
      requestAnimationFrame(() => revealSection(key))
    },
    [sections, setSelected, revealSection],
  )

  /** Where section focus was entered from: keyboard focus returns there on exit. */
  const focusOrigin = useRef<string | null>(null)
  const unfold = useCallback((key: string, tree: readonly PlanSection[] = sections) => {
    setCollapsed((current) => {
      const hiding = [key, ...ancestorKeys(tree, key)].filter((k) => current.has(k))
      if (hiding.length === 0) return current
      const next = new Set(current)
      for (const k of hiding) next.delete(k)
      return next
    })
  }, [sections])
  const enterFocus = useCallback((key: string) => {
    if (!onSectionFocus || !rows.some(row => row.key === key)) return
    if (!scopeRef.current) focusOrigin.current = selectedRef.current ?? key
    unfold(key)
    setSelectedQuietly(key)
    focused.current = key
    onSectionFocus(key, { section: key })
  }, [onSectionFocus, rows, unfold, setSelectedQuietly])
  /** Leave focus and land on `key` (selected, scrolled, keyboard focus on its heading). */
  const exitFocusTo = useCallback((key: string | null, tree: readonly PlanSection[] = sections) => {
    if (!onSectionFocus) return
    if (key) {
      unfold(key, tree)
      setSelectedQuietly(key)
      focused.current = key
    }
    onSectionFocus(null, key ? { section: key } : {})
  }, [onSectionFocus, sections, unfold, setSelectedQuietly])
  const exitFocus = useCallback(() => {
    const current = scopeRef.current
    if (!current) return
    const origin = focusOrigin.current
    exitFocusTo(origin && rows.some(row => row.key === origin) ? origin : current.root.key)
  }, [exitFocusTo, rows])
  /** Navigation to a section: stays focused when the target is inside the focus,
   *  leaves focus and goes there when it is not. */
  const goTo = useCallback((key: string) => {
    const current = scopeRef.current
    if (current && !current.ids.has(key)) exitFocusTo(key)
    else onSelect(key)
  }, [exitFocusTo, onSelect])

  // An unknown or deleted focus id: show the whole document, say so, drop the param.
  useEffect(() => {
    if (!sectionFocus || scope || !treeRead || !synced || rows.length === 0) return
    setNotice({ text: words.focusUnknown })
    onSectionFocus?.(null, { replace: true })
  }, [sectionFocus, scope, treeRead, synced, rows.length, onSectionFocus, words.focusUnknown])

  // The URL's last (selection, focus) pair: a selection that changes while the
  // focus stays put is a navigation (an agent's link, say) and may leave focus;
  // one that arrives together with a focus change (Back, a shared link) is not.
  const lastSectionFocus = useRef<{ section: string | null; focus: string | null } | null>(null)
  // A section handed over by link. Waits for the section to exist: the ask
  // arrives with the URL and the document is still syncing.
  useEffect(() => {
    const last = lastSectionFocus.current
    const focusKey = scope?.root.key ?? null
    lastSectionFocus.current = { section: focusSection, focus: focusKey }
    if (!focusSection) {
      // A missing URL selection is not a new instruction on every tree edit.
      if (focused.current !== null) {
        focused.current = null
        setSelected(null)
      }
      return
    }
    if (focused.current === focusSection) return
    if (!rows.some((row) => row.key === focusSection)) return
    if (scope && !scope.ids.has(focusSection)) {
      if (last && last.focus === focusKey && last.section !== focusSection) exitFocusTo(focusSection)
      else focused.current = focusSection
      return
    }
    if (activeMatch?.sectionId !== focusSection) onSelect(focusSection)
    focused.current = focusSection
  }, [focusSection, rows, onSelect, setSelected, activeMatch?.sectionId, scope, exitFocusTo])

  // Entering, changing and leaving focus move the reader: to the focused
  // section's heading, or back to where they came from. A shared link opened
  // cold only scrolls; it does not steal keyboard focus.
  const previousFocusKey = useRef<string | null | undefined>(undefined)
  const pendingReveal = useRef<{ key: string; focus: boolean } | null>(null)
  useEffect(() => {
    if (sectionFocus && !scope) return
    const now = scope?.root.key ?? null
    const before = previousFocusKey.current
    previousFocusKey.current = now
    if (before === now) return
    if (before === undefined) {
      if (now && !(focusSection && scope?.ids.has(focusSection))) pendingReveal.current = { key: now, focus: false }
      return
    }
    if (now) {
      unfold(now)
      pendingReveal.current = { key: now, focus: true }
      return
    }
    const target = focusSection && rows.some(row => row.key === focusSection) ? focusSection : before
    if (target) pendingReveal.current = { key: target, focus: true }
  }, [sectionFocus, scope, focusSection, rows, unfold])
  useEffect(() => {
    const reveal = pendingReveal.current
    if (!reveal) return
    const element = elements.current.get(reveal.key)
    if (!element) return
    pendingReveal.current = null
    revealSection(reveal.key)
    if (!reveal.focus) return
    const heading = element.querySelector<HTMLElement>('.document-heading')
    if (!heading) return
    if (heading.getAttribute('contenteditable') !== 'true' && !heading.hasAttribute('tabindex')) heading.tabIndex = -1
    heading.focus({ preventScroll: true })
  })

  // Alt+F focuses the current section (or leaves focus when it is the focused
  // one); Alt+Shift+F is the same for browsers that reserve Alt+F for their own
  // menu. Escape leaves focus when nothing else claimed it.
  useEffect(() => {
    if (!onSectionFocus) return
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.ctrlKey || event.metaKey) return
      const overlay = document.querySelector('[role="dialog"], [role="menu"], [data-slot="popover-content"]')
      if (event.altKey && event.code === 'KeyF') {
        const key = selectedRef.current
        if (!key || overlay) return
        event.preventDefault()
        if (scopeRef.current?.root.key === key) exitFocus()
        else enterFocus(key)
        return
      }
      if (event.key !== 'Escape' || event.altKey || event.shiftKey || !scopeRef.current || overlay) return
      const active = document.activeElement
      // Form fields and a title being edited own Escape; section prose does not
      // (its menus claim Escape themselves, which `defaultPrevented` reports).
      const editing = active instanceof HTMLElement && (isTextEntry(active) || !!active.closest('[contenteditable]:not([contenteditable="false"])'))
      if (editing && !(active as HTMLElement).closest('.ProseMirror')) return
      event.preventDefault()
      exitFocus()
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [onSectionFocus, enterFocus, exitFocus])

  // Work waiting outside the focus, read from the shared document only while focused.
  const [threads, setThreads] = useState<ReturnType<typeof readCommentThreads>>([])
  useEffect(() => {
    if (!scope) return
    const map = ydoc.getMap(COMMENT_FIELD)
    const read = () => setThreads(readCommentThreads(ydoc))
    read()
    map.observeDeep(read)
    return () => map.unobserveDeep(read)
  }, [ydoc, scope])

  const onToggleHistory = useCallback((key: string) => {
    const open = !openHistory.has(key)
    if (open) setCollapsed(current => { const next = new Set(current); next.delete(key); return next })
    onHistoryOpen?.(key, open)
    setOpenHistory(current => { const next = new Set(current); if (open) next.add(key); else next.delete(key); return next })
  }, [openHistory, onHistoryOpen])

  const onToggleProposals = useCallback((key: string) => {
    setCollapsed(current => { const next = new Set(current); next.delete(key); return next })
    setOpenProposals((current) => {
      const next = new Set(current)
      if (!next.delete(key)) next.add(key)
      return next
    })
  }, [])

  const visible = useMemo(() => visibleSections(scopeSections, effectiveCollapsed), [scopeSections, effectiveCollapsed])

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
    const create = () => insertPlanSection(ydoc, after, level, title, session.display)
    const key = history ? history.insert(create) : create()
    if (!key) return false
    pendingEditorFocus.current = key
    // Inside the focused subtree it simply appears; anywhere else (an H1 at the
    // top level, a sibling of the focused section) focus is left and the reader
    // follows the new section.
    const current = scopeRef.current
    if (current) {
      const fresh = planSections(readPlanTree(ydoc))
      if (!sectionFocusScope(fresh, current.root.key)?.ids.has(key)) {
        setNear((near) => near === null ? null : new Set([...near, key]))
        exitFocusTo(key, fresh)
        return true
      }
    }
    setCollapsed(new Set())
    setNear((current) => current === null ? null : new Set([...current, key]))
    setSelected(key)
    return true
  }

  const pendingPassage = useRef<SearchResult | null>(null)
  const passageRequest = useRef(0)
  const passageNoticeTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  useEffect(() => () => { clearTimeout(passageNoticeTimer.current); passageRequest.current++ }, [])
  const revealPassage = useCallback(async (editor: Editor, result: SearchResult) => {
    const request = ++passageRequest.current
    const source = editor.state.doc
    const range = await locatedPassageRange(source, result).catch(() => null)
    if (editor.isDestroyed || request !== passageRequest.current || selectedRef.current !== result.node_id) return
    if (range && editor.state.doc.eq(source)) {
      editor.chain().setTextSelection(range).focus(undefined, { scrollIntoView: false }).run()
      revealEditorPosition(editor, range.from)
      highlightSearchPassage(editor.view, range)
      const notice = { text: locale === 'de' ? 'Textstelle hervorgehoben.' : 'Passage highlighted.' }
      setNotice(notice)
      clearTimeout(passageNoticeTimer.current)
      passageNoticeTimer.current = setTimeout(() => setNotice(current => current === notice ? null : current), 3000)
    } else {
      editor.commands.focus('start')
      setNotice({ text: locale === 'de' ? 'Abschnitt geöffnet. Die Textstelle hat sich geändert oder ist nicht eindeutig.' : 'Section opened. The passage has changed or is ambiguous.' })
    }
  }, [locale])
  const editors = useRef(new Map<string, Editor>())
  const pendingBoundaryFocus = useRef<{ key: string; position: 'start' | 'end' } | null>(null)
  const [activeEditor, setActiveEditor] = useState<Editor | null>(null)
  const editingTitle = useRef(false)
  const editorRefs = useRef(new Map<string, (editor: Editor | null) => void>())
  const editorUnsubscribes = useRef(new Map<string, () => void>())
  useEffect(() => {
    const subscriptions = editorUnsubscribes.current
    return () => { for (const unsubscribe of subscriptions.values()) unsubscribe() }
  }, [])

  const commentsSection = comments?.section ?? null
  const commentsSectionRef = useRef(commentsSection)
  commentsSectionRef.current = commentsSection
  useEffect(() => {
    setCommentsEditor(commentsSection ? editors.current.get(commentsSection) ?? null : null)
  }, [commentsSection])
  useEffect(() => {
    const thread = pendingComment.current
    if (!thread || !commentsEditor || thread.sectionId !== commentsSection) return
    pendingComment.current = null
    const range = resolveCommentAnchor(commentsEditor, thread.anchor)
    if (range) { commentsEditor.commands.setTextSelection(range); commentsEditor.commands.focus(undefined, { scrollIntoView: false }); revealEditorPosition(commentsEditor, range.from) }
    else setNotice({ text: locale === 'de' ? 'Text geändert oder entfernt · Zitat im Kommentar erhalten' : 'Text changed or removed · quote retained in the comment' })
  }, [commentsEditor, commentsSection, comments, locale])
  const syncTextTools = useCallback(() => {
    const editor = !editingTitle.current && selectedRef.current ? editors.current.get(selectedRef.current) : undefined
    setActiveEditor(editor ?? null)
  }, [])
  useEffect(() => { syncTextTools() }, [selected, syncTextTools])
  const editorRefFor = useCallback((key: string) => {
    const existing = editorRefs.current.get(key)
    if (existing) return existing
    const fn = (editor: Editor | null) => {
      editorUnsubscribes.current.get(key)?.()
      editorUnsubscribes.current.delete(key)
      if (editor) {
        editors.current.set(key, editor)
        if (pendingPassage.current?.node_id === key) {
          const result = pendingPassage.current
          pendingPassage.current = null
          requestAnimationFrame(() => { if (!editor.isDestroyed) revealPassage(editor, result) })
        }
        if (commentsSectionRef.current === key) setCommentsEditor(editor)
        const update = () => { if (selectedRef.current === key) syncTextTools() }
        const focus = () => { editingTitle.current = false; update() }
        editor.on('transaction', update)
        editor.on('focus', focus)
        editorUnsubscribes.current.set(key, () => { editor.off('transaction', update); editor.off('focus', focus) })
        update()
        if (pendingEditorFocus.current === key) {
          pendingEditorFocus.current = null
          editor.commands.focus('start')
        }
        if (pendingBoundaryFocus.current?.key === key) {
          const position = pendingBoundaryFocus.current.position
          pendingBoundaryFocus.current = null
          editor.commands.focus(position)
        }
      }
      else {
        editors.current.delete(key)
        if (commentsSectionRef.current === key) setCommentsEditor(null)
        if (selectedRef.current === key) syncTextTools()
      }
    }
    editorRefs.current.set(key, fn)
    return fn
  }, [syncTextTools, revealPassage])

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
      if (history) history.record(() => setTitle(ydoc, key, text))
      else setTitle(ydoc, key, text)
      onEdited(key)
    },
    [canWrite, ydoc, onEdited, history],
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
  const [outlinePinned, setOutlinePinned] = useState(() => {
    try {
      return localStorage.getItem('takomo.plan.outline') !== 'closed'
    } catch {
      return true
    }
  })
  useEffect(() => {
    try {
      localStorage.setItem('takomo.plan.outline', outlinePinned ? 'open' : 'closed')
    } catch {
      // A private window refuses storage; the fold still works for this visit.
    }
  }, [outlinePinned])
  // Below DOCUMENT_PANE_WIDE the stylesheet lays the outline over the prose as a
  // drawer, so its behaviour follows the same measurement: the pane, not the
  // viewport, because a conversation beside the document narrows the pane alone.
  const paneRef = useRef<HTMLElement>(null)
  const [paneNarrow, setPaneNarrow] = useState(() => typeof window !== 'undefined' && window.innerWidth < DOCUMENT_PANE_WIDE)
  useEffect(() => {
    const pane = paneRef.current
    if (!pane || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(entries => {
      const width = entries[entries.length - 1]?.contentRect.width
      if (width) setPaneNarrow(width < DOCUMENT_PANE_WIDE)
    })
    observer.observe(pane)
    return () => observer.disconnect()
  }, [])
  const [outlineDrawer, setOutlineDrawer] = useState(false)
  useEffect(() => { setOutlineDrawer(false) }, [paneNarrow])
  const outlineOpen = paneNarrow ? outlineDrawer : outlinePinned
  const toggleOutline = () => { if (paneNarrow) setOutlineDrawer(v => !v); else setOutlinePinned(v => !v) }

  const [fragments, setFragments] = useState<Map<string, Y.XmlFragment>>(() => new Map())
  const known = useRef(fragments)
  useEffect(() => {
    const made = new Map<string, Y.XmlFragment>()
    for (const row of visible) {
      if (near !== null && !near.has(row.key) && row.key !== selected && row.key !== activeMatch?.sectionId) continue
      if (known.current.has(row.key)) continue
      const frag = canWrite ? proseOf(ydoc, row.key) : readProseOf(ydoc, row.key)
      if (frag) made.set(row.key, frag)
    }
    if (made.size === 0) return
    const next = new Map(known.current)
    for (const [key, frag] of made) next.set(key, frag)
    known.current = next
    setFragments(next)
  }, [visible, near, canWrite, ydoc, selected, activeMatch?.sectionId])

  // Unmounted sections keep their last mounted height (`lib/section-heights.ts`).
  // Synced before paint, so an editor leaving never shows a frame of shrinkage.
  const [sectionHeights] = useState(() => new SectionHeights())
  useEffect(() => () => sectionHeights.dispose(), [sectionHeights])
  useLayoutEffect(() => {
    sectionHeights.setZoom(zoom.value)
    for (const row of visible) {
      if (effectiveCollapsed.has(row.key)) continue
      const mounted = near === null || near.has(row.key) || row.key === selected || row.key === activeMatch?.sectionId
      sectionHeights.sync(row.key, mounted && fragments.has(row.key))
    }
  })

  useEffect(() => {
    if (!activeSearchSection) return
    setSelected(activeSearchSection)
  }, [activeSearchSection, setSelected])
  useEffect(() => {
    const targets = [...elements.current.entries()].flatMap(([key, element]) => {
      const heading = element.querySelector<HTMLElement>('.document-heading')
      return heading ? [{ element: heading, activeFrom: activeMatch?.kind === 'heading' && activeMatch.sectionId === key ? activeMatch.from : undefined }] : []
    })
    return highlightDocumentHeadings(targets, search.query)
  }, [search.query, activeMatch, visible])
  const activeSearchFragment = activeSearchSection ? fragments.get(activeSearchSection) : undefined
  useEffect(() => {
    if (!activeSearchSection || !activeSearchKey) return
    const frame = requestAnimationFrame(() => {
      const section = elements.current.get(activeSearchSection)
      const match = activeSearchKind === 'prose' ? section?.querySelector('[data-document-search-active="true"]') : section?.querySelector('.document-heading')
      revealInColumn((match ?? section) as HTMLElement | null | undefined)
    })
    return () => cancelAnimationFrame(frame)
  }, [activeSearchSection, activeSearchKind, activeSearchKey, activeSearchFragment, revealInColumn])
  const moveSection = (id: string, target: string, placement: SectionPlacement) => {
    if (!canWrite || !history) return { ok: false as const, error: 'changed' as const }
    const result = history.move(id, target, placement)
    if (result.ok) {
      const freshSections = planSections(readPlanTree(ydoc))
      setCollapsed(current => new Set([...current].filter(key => ![id, ...ancestorKeys(freshSections, id)].includes(key))))
      setNotice({ text: locale === 'de' ? 'Abschnitt verschoben' : 'Section moved', undo: true })
      const current = scopeRef.current
      if (current && !sectionFocusScope(freshSections, current.root.key)?.ids.has(id)) exitFocusTo(id, freshSections)
      else setSelected(id)
      onMoved?.(id)
    }
    return result
  }
  const moveHistory = (direction: 'undo' | 'redo') => {
    if (!canWrite || !history) return
    const key = direction === 'undo' ? history.undoSection : history.redoSection
    const result = history[direction]()
    if (result.ok && key) {
      const ancestors = [key, ...ancestorKeys(planSections(readPlanTree(ydoc)), key)]
      setCollapsed(current => new Set([...current].filter(id => !ancestors.includes(id))))
      setSelected(key)
      onMoved?.(key)
      setNotice({ text: locale === 'de' ? 'Verschiebung aktualisiert' : 'Section move updated' })
    } else if (!result.ok) setNotice({ text: locale === 'de' ? 'Die Abschnittsstruktur wurde inzwischen geändert. Diese Aktion ist nicht mehr verfügbar.' : 'The section structure has changed. This action is no longer available.' })
  }
  const focusProse = (key: string, position: 'start' | 'end'): boolean => {
    if (!canWrite) return false
    setSelected(key)
    setCollapsed(current => { const next = new Set(current); next.delete(key); return next })
    const editor = editors.current.get(key)
    if (editor) editor.commands.focus(position)
    else pendingBoundaryFocus.current = { key, position }
    return true
  }
  const focusTitle = (key: string, position: 'start' | 'end'): boolean => {
    const heading = elements.current.get(key)?.querySelector<HTMLElement>('.document-heading[contenteditable="true"]')
    if (!canWrite || !heading) return false
    setSelected(key)
    heading.focus()
    const range = document.createRange()
    range.selectNodeContents(heading)
    range.collapse(position === 'start')
    window.getSelection()?.removeAllRanges()
    window.getSelection()?.addRange(range)
    return true
  }
  const knownKeys = useMemo(() => new Set(rows.map(row => row.key)), [rows])
  const outsideCounts = useMemo(() => scope ? outsideFocus(scope, knownKeys, pending, threads) : { proposals: 0, comments: 0 }, [scope, knownKeys, pending, threads])
  const numberShown = (section: PlanSection) => section.depth === 0 ? numbering.value.h1 : section.depth === 1 ? numbering.value.h2 : true
  const sectionLabel = (section: PlanSection) => `${numberShown(section) ? `${section.number} ` : ''}${section.title || railLabels.untitled}`
  const focusSectionLabels = useMemo(() => ({ ...sectionLabels, focusSection: words.focusSection }), [sectionLabels, words.focusSection])
  return (
    <DocumentReviewProvider key={`${token}:${session.mindmap}`} token={token ?? ""} map={session.mindmap} project={project ?? ""} locale={locale} canWrite={canWrite}><main ref={paneRef} className="@container/document-pane flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
      <DocumentActions focusMode={focusMode} locale={locale} canWrite={canWrite}
        canUndo={history?.canUndo ?? false} canRedo={history?.canRedo ?? false}
        onUndo={() => moveHistory('undo')} onRedo={() => moveHistory('redo')} primary={<>        <button
          type="button"
          onClick={toggleOutline}
          hidden={focusMode}
          aria-controls="document-outline"
          onMouseDown={event => event.preventDefault()}
          aria-expanded={outlineOpen}
          className="text-muted-foreground hover:text-foreground flex min-h-8 items-center gap-1.5 self-start rounded-md px-1.5 py-1 text-[12px] font-[650]"
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
        {token && <DocumentEmbeddingStatus locale={locale} canSync={canWrite} />}
        <DocumentZoomControl zoom={zoom} locale={locale} /></>} >
        {token && <DocumentHybridSearch key={`${session.mindmap}:${token}`} scope={scope} token={token} map={session.mindmap} userId={userId} project={project} locale={locale} canSync={canWrite} onNavigate={result => {
          if (!rows.some(section => section.key === result.node_id)) {
            setNotice({ text: locale === 'de' ? 'Dieser Abschnitt wurde entfernt. Bitte erneut suchen.' : 'This section was removed. Search again.' })
            return
          }
          goTo(result.node_id)
          setOutlineDrawer(false)
          setNear(current => current === null ? null : new Set([...current, result.node_id]))
          const editor = editors.current.get(result.node_id)
          if (editor) requestAnimationFrame(() => { if (!editor.isDestroyed) revealPassage(editor, result) })
          else pendingPassage.current = result
        }} />}
        {agentTools}
        <DocumentZoomSteps zoom={zoom} locale={locale} />
        <DocumentSectionReferenceButton editor={activeEditor} ydoc={ydoc} locale={locale} canWrite={canWrite} />
        <DocumentCommentButton editor={activeEditor} locale={locale} canWrite={canWrite}
          onComment={draft => { if (selected) setComments({ section: selected, draft }) }} />
        <button type="button" hidden={focusMode} className="rounded px-2 py-1 text-sm hover:bg-muted" aria-expanded={allComments} onClick={() => setAllComments(value => !value)}>{locale === 'de' ? 'Alle Kommentare' : 'All comments'}</button>
      </DocumentActions>
      {notice && <div role="status" className="flex flex-none items-center gap-3 bg-muted px-4 py-2 text-sm">
        <span>{notice.text}</span>{notice.undo && <button type="button" className="underline" onClick={() => moveHistory('undo')}>{locale === 'de' ? 'Rückgängig' : 'Undo'}</button>}
      </div>}
      {scope && <SectionFocusBand
        label={words.focusRegion}
        text={words.focusBand.replace('{title}', sectionLabel(scope.root))}
        exit={words.focusExit}
        exitHint={words.focusExitHint}
        outside={[
          outsideCounts.proposals === 1 ? words.focusProposalOutside : outsideCounts.proposals > 1 ? words.focusProposalsOutside.replace('{n}', String(outsideCounts.proposals)) : '',
          outsideCounts.comments === 1 ? words.focusCommentOutside : outsideCounts.comments > 1 ? words.focusCommentsOutside.replace('{n}', String(outsideCounts.comments)) : '',
        ].filter(Boolean)}
        onExit={exitFocus}
      />}
      {moving && canWrite && <MoveSectionDialog sections={sections} sectionKey={moving} lang={locale} onClose={() => setMoving(null)} onMove={moveSection} />}
      <div className="relative flex min-h-0 flex-1 flex-col overflow-hidden @min-[850px]/document-pane:flex-row">
      {/* The outline follows the available document pane, including when the
          conversation takes half of a wide viewport. */}
      {/* Collapsible, and the state is remembered.
          On a long plan the outline is how you navigate; on a narrow window it
          is competing with the prose for the only column that matters. Both are
          true at different moments, so hiding it frees the entire column.
          A compact toolbar control reopens it without retaining a sidebar strip. */}
      <aside
        id="document-outline"
        style={{ display: focusMode || !outlineOpen ? 'none' : undefined }}
        className="document-outline border-b-border-soft absolute inset-x-0 top-0 z-40 flex max-h-[80%] flex-none flex-col overflow-y-auto border-b bg-white px-2 py-2 shadow-lg @min-[850px]/document-pane:static @min-[850px]/document-pane:max-h-none @min-[850px]/document-pane:w-80 @min-[850px]/document-pane:flex-none @min-[850px]/document-pane:resize-x @min-[850px]/document-pane:border-r @min-[850px]/document-pane:border-b-0 @min-[850px]/document-pane:shadow-none dark:bg-card"
      >
        {outlineOpen && (
          <OutlineRail numbering={numbering.value} locale={locale} onReorder={canWrite ? moveSection : undefined}
            sections={scopeSections}
            onFocusSection={onSectionFocus ? key => { enterFocus(key); if (paneNarrow) setOutlineDrawer(false) } : undefined}
            selected={selected}
            onSelect={key => { onSelect(key); if (paneNarrow) setOutlineDrawer(false) }}
            collapsible={collapsible}
            collapsed={effectiveCollapsed}
            onToggle={onToggleFold}
            standing={standings}
            pending={pending}
            labels={{ ...railLabels, move: locale === 'de' ? 'Abschnitt verschieben' : 'Move section', focusSection: words.focusSection }}
            onMove={canWrite ? setMoving : undefined}
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
              <DocumentStart locale={locale} onInsert={(level, title) => insertSection(null, level, title)} />
            ) : <p>{labels.empty}</p>}
          </div>
        ) : (
          // A measure, not a width: prose running the full width of a desktop
          // window is unreadable, and a `max-width` cap cannot overflow a phone.
          // The cap and the page padding live on `.document-page` in
          // `styles/editor.css`; `mx-auto` centres that measure in whatever
          // column is left once the outline has taken its share.
          <div className="document-appearance document-page mx-auto min-w-0"
            style={{ ...documentAppearanceStyle(appearance), ...zoom.style }}>
            {scope && <SectionFocusBreadcrumb
              label={words.focusBreadcrumb}
              root={words.focusRoot}
              ancestors={scope.ancestors.map(section => ({ key: section.key, label: sectionLabel(section) }))}
              current={sectionLabel(scope.root)}
              onRoot={exitFocus}
              onAncestor={key => enterFocus(key)}
            />}
            {visible.map((row, rowIndex) => {
              const folded = effectiveCollapsed.has(row.key)
              const mounted = !folded && (near === null || near.has(row.key) || row.key === selected || row.key === activeMatch?.sectionId)
              const fragment = mounted ? (fragments.get(row.key) ?? null) : null
              const preview = previews.get(row.key) ?? ''
              const offered = proposalsFor.get(row.key) ?? []
              return (
                <SectionPanel
                  key={row.key}
                  number={row.number}
                  depth={row.depth}
                  showNumber={row.depth === 0 ? numbering.value.h1 : row.depth === 1 ? numbering.value.h2 : true}
                  title={row.title}
                  collapsed={folded}
                  onToggleCollapse={collapsible.has(row.key) ? () => onToggleFold(row.key) : undefined}
                  collapseLabel={folded ? railLabels.expand : railLabels.collapse}
                  summary={<p className="text-muted-foreground mb-3 whitespace-pre-wrap break-words text-sm">{summaries[row.key]}</p>}
                  onTitle={(text) => onTitle(row.key, text)}
                  onHeadingEnter={() => { focusProse(row.key, 'start') }}
                  onHeadingDown={() => focusProse(row.key, 'start')}
                  onHeadingFocus={() => { editingTitle.current = true; syncTextTools() }}
                  onHeadingUp={() => rowIndex > 0 && focusProse(visible[rowIndex - 1]!.key, 'end')}
                  headingLink={<CopySectionLink href={new URL(specificationLink(project, 'document', row.key), window.location.origin).href} locale={locale} />}
                  headingActions={<>
                    {ticketLinksFor?.(row.key)}
                    <button type="button" className="text-muted-foreground hover:text-foreground" onClick={() => { onSelect(row.key); setComments({ section: row.key, draft: null }) }}>
                      {locale === 'de' ? 'Kommentare' : 'Comments'}
                    </button>
                  </>}
                  standing={standings[row.key] ?? 'unseen'}
                  entries={trace.get(row.key) ?? []}
                  historyOpen={openHistory.has(row.key)}
                  historyLoading={traceState?.[row.key]?.loading}
                  historyError={traceState?.[row.key]?.error}
                  onHistoryRetry={() => onHistoryRetry?.(row.key)}
                  onToggleHistory={() => onToggleHistory(row.key)}
                  onReview={() => onReview(row.key)}
                  onShowOnMap={() => onShowOnMap(row.key)}
                  onFocusSection={onSectionFocus && scope?.root.key !== row.key ? () => enterFocus(row.key) : undefined}
                  onShowTests={() => onShowTests(row.key)}
                  testsStatus={testsFor(row.key).total > 0 ? (
                    <TestsStatusLine
                      counts={testsFor(row.key)}
                      verifiedLabel={verifiedLabel}
                      failedLabel={failedLabel}
                      label={testsLabel}
                      onOpen={() => onShowTests(row.key)}
                    />
                  ) : undefined}
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
                  onActivate={() => { focused.current = row.key; setSelected(row.key) }}
                  sectionRef={refFor(row.key)}
                  labels={focusSectionLabels}
                >
                  {/* The slot holds the section's last mounted height while it
                      shows the preview (`lib/section-heights.ts`). */}
                  <div ref={sectionHeights.slotRef(row.key)} data-prose-slot="">
                  <div>
                  {fragment ? (
                    <SectionEditor
                      locale={locale}
                      history={history ?? undefined}
                      ydoc={ydoc}
                      sectionId={row.key}
                      onFollowReference={goTo}
                      onOpenComments={() => { setSelected(row.key); setComments({ section: row.key, draft: null }) }}
                      fragment={fragment}
                      provider={provider}
                      display={session.display}
                      color={color}
                      canWrite={canWrite}
                      maxSectionLevel={Math.min(row.depth + 2, 3)}
                      sectionSummary={summaries[row.key]}
                      onSetSectionSummary={summary => canWrite && setSectionSummary(ydoc, row.key, summary)}
                      onRemoveSectionSummary={() => {
                        if (!canWrite || !removeSectionSummary(ydoc, row.key)) return false
                        setCollapsed(current => { const next = new Set(current); next.delete(row.key); return next })
                        return true
                      }}
                      onInsertSection={(level, title) => insertSection(row.key, level, title)}
                      onNavigate={target => target === 'title' ? focusTitle(row.key, 'end') :
                        !!visible[rowIndex + 1] && focusTitle(visible[rowIndex + 1]!.key, 'start')}
                      onSettled={() => onEdited(row.key)}
                      highlight={highlights[row.key] ?? ''}
                      searchQuery={search.query}
                      searchActiveFrom={activeMatch?.kind === 'prose' && activeMatch.sectionId === row.key ? activeMatch.from : undefined}
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
                  </div>
                  </div>
                  <SectionReferences ydoc={ydoc} sectionId={row.key} title={row.title} locale={locale} canWrite={canWrite}
                    onChange={change => { if (!canWrite) return; if (history) history.record(change); else change(); onEdited(row.key) }} />
                  <div className="section-discussion">{conversationFor?.(row.key)}</div>
                  {comments?.section === row.key && <DocumentComments key={row.key} ydoc={ydoc} sectionId={row.key}
                    editor={commentsEditor} actor={session.display} locale={locale} canWrite={canWrite}
                    draft={comments.draft} onDraftConsumed={() => setComments(current => current?.section === row.key ? { section: row.key, draft: null } : current)}
                    onClose={() => setComments(null)} />}
                </SectionPanel>
              )
            })}
          </div>
        )}
      </div>
      {allComments && !focusMode && <aside className="absolute inset-0 z-40 min-w-0 overflow-y-auto border-border-soft bg-card @min-[850px]/document-pane:static @min-[850px]/document-pane:w-80 @min-[850px]/document-pane:flex-none @min-[850px]/document-pane:border-l">
        <DocumentComments ydoc={ydoc} editor={null} actor={session.display} locale={locale} canWrite={canWrite}
          sectionTitle={id => { const row = rows.find(item => item.key === id); return row ? row.title || railLabels.untitled : null }}
          onShowThread={thread => { pendingComment.current = thread; goTo(thread.sectionId); setComments({ section: thread.sectionId, draft: null }); setAllComments(false) }}
          onDraftConsumed={() => {}} onClose={() => setAllComments(false)} />
      </aside>}
      </div>
    </main></DocumentReviewProvider>
  )
}

/**
 * A section's verification at a glance, under its heading: how many of the
 * behaviors that name it are verified, and whether any fail. It opens the
 * Tests view on that section; the detail lives there.
 */
function TestsStatusLine({
  counts,
  verifiedLabel,
  failedLabel,
  label,
  onOpen,
}: {
  counts: { total: number; failing: number; verified?: number }
  verifiedLabel: string
  failedLabel: string
  label: string
  onOpen: () => void
}) {
  const failing = counts.failing > 0
  const done = !failing && counts.verified === counts.total
  const Icon = failing ? ShieldAlert : ShieldCheck
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-label={`${label}: ${counts.verified ?? 0}/${counts.total} ${verifiedLabel}${failing ? `, ${counts.failing} ${failedLabel}` : ''}`}
      className={cn(
        'mb-2 inline-flex cursor-pointer items-center gap-1.5 rounded-full border px-2 py-0.5 text-xs hover:underline',
        failing ? 'border-nfbd bg-nfbg text-nf' : done ? 'border-okbd bg-okbg text-ok' : 'text-muted-foreground',
      )}
    >
      <Icon className="size-3.5 shrink-0" aria-hidden="true" />
      <span className="tabular-nums">
        {counts.verified ?? 0}/{counts.total} {verifiedLabel}
      </span>
      {failing && (
        <span className="font-semibold tabular-nums">
          · {counts.failing} {failedLabel}
        </span>
      )}
      <ChevronRight className="size-3 shrink-0" aria-hidden="true" />
    </button>
  )
}
