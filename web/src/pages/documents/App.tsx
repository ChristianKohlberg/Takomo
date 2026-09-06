import { DiagramContext } from '@/lib/diagram'
import { useProjectUpdates } from '@/hooks/useProjectUpdates'
import { useWorkspaceNavigate } from '@/hooks/useWorkspace'
import { useWorkspaceSection } from '@/hooks/useWorkspaceSection'
import { useSpecification } from '../specification/context'
// /documents — the project's plan, written out.
//
// A project has ONE plan. The map and this page are two renderings of it, not
// two things kept in step: a node is a section, its title is the heading, its
// depth is the heading level, and tree order is reading order
// (`spec/one-model-two-views.md`). The canvas is for growing and grouping
// thoughts fast; this is where they are spelled out, and where the history of
// who wrote and who agreed is visible.
//
// What used to be here was a FILE BROWSER over `documents` rows, filled by
// converting a map into a tree of them. That conversion is gone, and the reason
// it had to go is worth keeping written down: a node's notes and its document's
// prose were two places one paragraph lived, and they disagreed after the first
// edit. Linking two copies is not the same as having one thing.
//
// The specification workspace owns the shared replica and shell. This view
// owns the plan's review history and document actions.
import { lazy, useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { SectionConversation } from '@/components/documents/SectionConversation'
import { useToast } from '@/components/Toaster'
import { pick } from '@/lib/i18n'
import {
  getMindmap,
  getTrace,
  recordTrace,
  type PlanStanding,
  type TraceEntry,
} from '@/lib/mindmaps'
import { traceByNode } from '@/lib/plan-trace'
import { mapLink } from '@/lib/plan-url'
import { STR } from './strings'

// Tiptap, ProseMirror, Yjs and the socket together outweigh the rest of the app,
// and every other surface would pay for them on first paint. This must stay a
// lazy import.
const Plan = lazy(() => import('./Plan'))

/** How much of the plan's history one read brings back. The server's ceiling is
 *  500; a plan is capped at 500 sections, so this is a page, not everything. */
const TRACE_LIMIT = 500

export function DocumentView() {
  const {
    token,
    scopes,
    lang,
    project,
    projects,
    focusMode,
    structureHistory,
    map,
    session,
    connection,
    onError: handleErr,
    openTests,
    testsFor,
  } = useSpecification()
  const { toast } = useToast()
  const navigate = useWorkspaceNavigate()
  const [focusSection, selectSection] = useWorkspaceSection()
  const historyEpoch = useRef(0)
  useEffect(() => {
    const ref = historyEpoch
    return () => {
      ref.current++
    }
  }, [])
  const [standing, setStanding] = useState<PlanStanding>({})
  const [entries, setEntries] = useState<TraceEntry[]>([])

  const t = useMemo(() => pick(STR, lang), [lang])

  const mapId = map?.id ?? null
  const refreshHistory = useCallback(async () => {
    if (!mapId) return
    const epoch = ++historyEpoch.current
    const [detail, page] = await Promise.all([
      getMindmap(token, mapId),
      getTrace(token, mapId, { limit: TRACE_LIMIT }),
    ])
    if (epoch !== historyEpoch.current) return
    setStanding(detail.standing ?? {})
    setEntries(page.items)
  }, [token, mapId])

  useProjectUpdates(token, project, async () => {
    try {
      await refreshHistory()
    } catch (error) {
      handleErr(error)
    }
  })

  useEffect(() => {
    if (!token || !mapId) return
    refreshHistory().catch(handleErr)
  }, [token, mapId, refreshHistory, handleErr])

  /**
   * Bring the history back in line, once, after a burst of writes.
   *
   * Somebody typing in three sections files three `edited` entries, and each of
   * them would otherwise refetch the whole plan's history. The trace is sparse
   * by contract; the reads of it should be too.
   */
  const pending = useRef<ReturnType<typeof setTimeout> | null>(null)
  const scheduleRefresh = useCallback(() => {
    if (pending.current) clearTimeout(pending.current)
    pending.current = setTimeout(() => {
      pending.current = null
      refreshHistory().catch(handleErr)
    }, 1500)
  }, [refreshHistory, handleErr])

  useEffect(
    () => () => {
      if (pending.current) clearTimeout(pending.current)
    },
    [],
  )

  const onReview = useCallback(
    (node: string) => {
      if (!mapId) return
      recordTrace(token, mapId, { kind: 'reviewed', node })
        .then(() => scheduleRefresh())
        .catch(handleErr)
    },
    [token, mapId, scheduleRefresh, handleErr],
  )

  const onEdited = useCallback(
    (node: string) => {
      if (!mapId) return
      recordTrace(token, mapId, { kind: 'edited', node })
        .then(() => scheduleRefresh())
        .catch(handleErr)
    },
    [token, mapId, scheduleRefresh, handleErr],
  )

  /** The map is where a section is named, moved and pruned. This hands it over
   *  by link, and the canvas selects and centres what arrives. */
  const onShowOnMap = useCallback(
    (node: string) => {
      if (!mapId) return
      navigate(mapLink(mapId, node))
    },
    [navigate, mapId],
  )

  /**
   * A decision on an agent's proposal, filed as an act on the section.
   *
   * The plan applies the change itself — the browser is the only place that can,
   * because markdown becomes nodes in the editor's own schema — so the server
   * never sees the edit as a request. What it gets is the record of who decided
   * what, which is the half that has to survive compaction.
   */
  const onDecided = useCallback(
    (node: string, kind: 'accepted' | 'rejected') => {
      if (!mapId) return
      recordTrace(token, mapId, { kind, node })
        .then(() => scheduleRefresh())
        .catch(handleErr)
    },
    [token, mapId, scheduleRefresh, handleErr],
  )

  /** Ops an accepted proposal could not apply, said out loud. A reviewer who
   *  believes they accepted the whole change has not reviewed it. */
  const onSkipped = useCallback(
    (messages: string[]) => {
      toast(
        `${t.applySkipped.replace('{n}', String(messages.length))} ${messages.join('; ')}`,
        'err',
      )
    },
    [toast, t],
  )

  const trace = useMemo(() => traceByNode(entries), [entries])

  if (!session || !connection) return null
  return (
    <DiagramContext value={{ token, project: map?.project ?? project }}>
    <Plan
      project={project}
      key={session.session}
      conversationFor={(node) => (
        <SectionConversation token={token} map={session.mindmap} node={node} lang={lang}
          canAsk={scopes.includes('human') && scopes.includes('write')} onError={handleErr} />
      )}
      focusMode={focusMode}
      structureHistory={structureHistory}
      appearance={projects.find((item) => item.id === project)?.document_appearance}
      locale={lang}
      session={session}
      connection={connection}
      testsFor={testsFor}
      onShowTests={openTests}
      testsLabel={t.viewTests}
      failedLabel={lang === 'de' ? 'fehlgeschlagen' : 'failing'}
      onError={handleErr}
      standing={standing}
      trace={trace}
      onReview={onReview}
      onEdited={onEdited}
      onMoved={scheduleRefresh}
      onShowOnMap={onShowOnMap}
      onDecided={onDecided}
      onSkipped={onSkipped}
      focusSection={focusSection}
      onSelection={selectSection}
      labels={{
        readOnly: t.readOnlyBanner,
        empty: t.empty,
        emptyHint: t.emptyHint,
        proseEmpty: t.proseEmpty,
        proseLabel: t.proseLabel,
      }}
      railLabels={{
        outline: t.outline,
        expand: t.outlineExpand,
        collapse: t.outlineCollapse,
        folded: t.outlineFolded,
        untitled: t.untitled,
        standingConfirmed: t.standingConfirmed,
        standingChanged: t.standingChanged,
        standingUnseen: t.standingUnseen,
        pending: t.railPending,
      }}
      sectionLabels={{
        actions: lang === 'de' ? 'Abschnittsaktionen' : 'Section actions',
        renameSection: t.renameSection,
        untitled: t.untitled,
        standingConfirmed: t.standingConfirmed,
        standingChanged: t.standingChanged,
        standingUnseen: t.standingUnseen,
        review: t.review,
        reviewHint: t.reviewHint,
        showOnMap: t.showOnMap,
        history: t.history,
        hideHistory: t.hideHistory,
        historyEmpty: t.historyEmpty,
        historyMore: t.historyMore,
        proposals: t.proposals,
        hideProposals: t.hideProposals,
        pendingBadge: t.pendingBadge,
        needWrite: t.needWrite,
        kinds: {
          authored: t.kindAuthored,
          renamed: t.kindRenamed,
          edited: t.kindEdited,
          moved: t.kindMoved,
          pruned: t.kindPruned,
          reviewed: t.kindReviewed,
          proposed: t.kindProposed,
          accepted: t.kindAccepted,
          rejected: t.kindRejected,
        },
      }}
      proposalLabels={{
        heading: t.proposalsHeading,
        empty: t.proposalsEmpty,
        pending: t.proposalPending,
        accepted: t.proposalAccepted,
        rejected: t.proposalRejected,
        accept: t.accept,
        reject: t.reject,
        by: t.proposalBy,
        partial: t.proposalPartial,
        opReplace: t.opReplace,
        opInsert: t.opInsert,
        opDelete: t.opDelete,
        readOnly: t.proposalReadOnly,
      }}
    />
    </DiagramContext>
  )
}
