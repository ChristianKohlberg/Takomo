import { useWorkspaceNavigate } from '@/hooks/useWorkspace'
import { useWorkspaceSection } from '@/hooks/useWorkspaceSection'
import { useSpecification } from '../specification/context'
// /mindmaps — brainstorming, before any of it is an idea, with everyone in the
// room at once.
//
// Each project has one plan. The shared navigation rail owns project switching;
// the canvas header shows the plan title, connection, and collaborators.
//
// Everything else is ⌘K, scoped to the selected node or to the map. That is not
// a shortcut for the toolbar: it is where the toolbar went. A canvas has one
// scarce resource, which is the canvas, and a command you summon costs none of it.
//
// The state source changed the page's job once already, and that still holds. The
// map used to be rows: every keystroke was a REST write, every write was followed
// by a refetch, and the page carried an optimistic tree so typing did not wait on
// a round trip. It is now a CRDT — one replica shared by every browser and every
// agent — so there is no optimistic copy to keep, no refetch, no save button and
// no duplicate tree to reconcile. The specification workspace owns the shared
// replica; this view owns the canvas commands.
import { useCallback, useEffect, useMemo, useState } from 'react'

import { CommandPalette } from '@/components/mindmap/CommandPalette'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { pick } from '@/lib/i18n'
import { fuzzyRank, isTextEntry } from '@/lib/mindmap-commands'
import { createMindmap, deleteMindmap, patchMindmap, promoteNode } from '@/lib/mindmaps'
import { planLink } from '@/lib/plan-url'
import { saveProject } from '@/lib/session'
import Live from './Live'
import { STR } from './strings'

export function MapView() {
  const {
    token,
    lang,
    project,
    projects,
    scopes,
    voice,
    map: open,
    session,
    connection,
    refreshMap,
    selectProject: setProject,
    onError: handleErr,
    openTests,
    testsFor,
  } = useSpecification()
  const navigate = useWorkspaceNavigate()
  const { toast } = useToast()
  const [focusNode, selectSection] = useWorkspaceSection()
  const openId = open?.id ?? null
  const selectedProject = project
  const refreshList = refreshMap
  const canWrite = scopes.includes('write')
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [paletteStage, setPaletteStage] = useState<'commands' | 'project'>('commands')
  const [paletteQuery, setPaletteQuery] = useState('')
  const [paletteActive, setPaletteActive] = useState<string | null>(null)

  const t = useMemo(() => pick(STR, lang), [lang])
  const onLiveError = useCallback((message: string) => toast(message, 'err'), [toast])
  const promote = useCallback(
    (node: string, target: 'epic' | 'initiative') => {
      if (!openId) return
      // Promotion goes over REST on purpose: it creates an epic or an initiative,
      // which is work in the store rather than a change to this document. The
      // server writes the node's link into the same room, so it arrives here over
      // the socket like any other edit.
      promoteNode(token, openId, node, target)
        .then(({ created }) => {
          toast(
            (target === 'epic' ? t.promotedEpic : t.promotedInitiative).replace('{id}', created.id),
            'success',
          )
        })
        .catch(handleErr)
    },
    [openId, token, toast, t, handleErr],
  )

  const newMap = useCallback(
    async (forProject?: string) => {
      if (!canWrite) {
        toast(t.needWrite, 'err')
        return
      }
      const target = forProject || selectedProject
      if (!target) {
        toast(t.needProject, 'err')
        return
      }
      const title = window.prompt(t.newMapPrompt)
      if (!title?.trim()) return
      try {
        await createMindmap(token, { project: target, title: title.trim() })
        await refreshList()
      } catch (e) {
        handleErr(e)
      }
    },
    [canWrite, toast, t, selectedProject, token, refreshList, handleErr],
  )

  const renameMap = useCallback(() => {
    if (!open) return
    const title = window.prompt(t.renameMapPrompt, open.title)
    if (!title?.trim()) return
    patchMindmap(token, open.id, { title: title.trim() })
      .then(() => refreshList())
      .catch(handleErr)
  }, [open, t, token, refreshList, handleErr])

  const removeMap = useCallback(() => {
    if (!open) return
    if (!window.confirm(t.confirmDeleteMap)) return
    deleteMindmap(token, open.id)
      .then(() => refreshList())
      .then(() => {
        toast(t.mapDeleted, 'success')
      })
      .catch(handleErr)
  }, [open, t, token, refreshList, toast, handleErr])

  /**
   * The way to the other rendering of this plan.
   *
   * The project is saved on the way through, because `/documents` shows the plan
   * of the SELECTED project — a project holds exactly one — and arriving at
   * somebody else's plan because the picker still pointed elsewhere would be a
   * hand-off to the wrong document. The section rides in the hash, honoured once
   * and then cleared, exactly as `#n=` is in the other direction.
   */
  const openPlan = useCallback(
    (node: string | null) => {
      if (open) {
        saveProject(open.project)
      }
      navigate(planLink(node))
    },
    [open, navigate],
  )

  /** The tests for the selected thought, on the same terms as the plan link. */
  const chooseProject = setProject

  // The empty-state palette. `Live` owns the shortcut whenever a map is open, so
  // this listener stands down rather than competing with it.
  useEffect(() => {
    if (open) return
    const onKey = (e: KeyboardEvent) => {
      if (e.key.toLowerCase() !== 'k' || !(e.metaKey || e.ctrlKey)) return
      if (!paletteOpen && isTextEntry(document.activeElement)) return
      e.preventDefault()
      setPaletteOpen((current) => {
        if (!current) {
          setPaletteStage('commands')
          setPaletteQuery('')
          setPaletteActive(null)
        }
        return !current
      })
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [open, paletteOpen])

  const emptyItems = useMemo(() => {
    if (paletteStage === 'project') {
      return fuzzyRank(projects, (p) => p.name || p.id, paletteQuery, 12).map((p) => ({
        id: p.id,
        label: p.name || p.id,
        hint: p.id === selectedProject ? t.paletteScopeMap : p.id,
      }))
    }
    const rows: { id: string; label: string; hint?: string }[] = []
    if (canWrite && selectedProject) rows.push({ id: 'map.new', label: t.cmdNewMap })
    if (projects.length > 1) rows.push({ id: 'map.project', label: t.cmdProject })
    return fuzzyRank(rows, (r) => r.label, paletteQuery, 12)
  }, [paletteStage, projects, paletteQuery, selectedProject, t, canWrite])

  return (
    <>
      <main className="flex min-h-0 flex-1 flex-col">
        {open ? (
          session ? (
            <Live
              key={session.session}
              session={session}
              connection={connection!}
              title={open.title}
              onError={onLiveError}
              projects={projects.map((p) => ({ id: p.id, name: p.name || p.id }))}
              currentProject={open.project}
              onProject={chooseProject}
              canManageMap={canWrite}
              onOpenPlan={openPlan}
              onOpenTests={openTests}
              testsFor={testsFor}
              token={token}
              voiceEnabled={voice}
              voiceLabels={{
                start: t.voiceStart,
                stop: t.voiceStop,
                starting: t.voiceStarting,
                hearing: t.voiceHearing,
                noMic: t.voiceNoMic,
                lost: t.voiceLost,
              }}
              onRenameMap={renameMap}
              focusNode={focusNode}
              onSelection={selectSection}
              onDeleteMap={removeMap}
              onPromote={promote}
              labels={{
                branch: t.branch,
                readOnly: t.readOnlyBanner,
                newThought: t.newThought,
                relationLabelPrompt: t.relationLabelPrompt,
                capNodes: t.capNodes,
                capRelationships: t.capRelationships,
                needWrite: t.needWrite,
                gotoPlaceholder: t.gotoPlaceholder,
                projectPlaceholder: t.projectPlaceholder,
                droppedFileGist: t.droppedFileGist,
                attachmentsFull: t.attachmentsFull,
                newQuestion: t.newQuestion,
                trustLens: t.trustLens,
                openPlan: t.openPlan,
              }}
              canvasLabels={{
                empty: t.canvasEmpty,
                emptyHint: t.canvasEmptyHint,
                fit: t.fit,
                tidy: t.tidy,
                radial: t.layoutRadial,
                tree: t.layoutTree,
                zoomIn: t.zoomIn,
                zoomOut: t.zoomOut,
                expand: t.expandBranch,
                collapse: t.collapseBranch,
                cannotDrop: t.cannotDrop,
                pickRelationTarget: t.pickRelationTarget,
                attachments: t.attachmentsBadge,
                addChild: t.addChild,
                nodeActions: t.nodeActions,
                nodeMenu: t.nodeMenu,
                dropHere: t.dropHere,
                trustLens: t.trustLens,
                trustLegend: t.trustLegend,
                trustConfirmed: t.trustConfirmed,
                trustMachine: t.trustMachine,
                trustUnverified: t.trustUnverified,
                cutEdge: t.cutEdge,
                nameField: t.nameField,
                nameHint: t.nameHint,
              }}
              outlineLabels={{
                edit: t.editThought,
                rename: t.renameThought,
                nameField: t.nameField,
                nameHint: t.nameHint,
                addChild: t.addChild,
                addSibling: t.addSibling,
                empty: t.canvasEmptyHint,
                hasNotes: t.hasNotes,
                attachments: t.attachmentsBadge,
                remove: t.cmdDelete,
                detach: t.detachRow,
                folded: t.foldedSummary,
                question: t.questionEyebrow,
                trustConfirmed: t.trustConfirmed,
                trustMachine: t.trustMachine,
                trustUnverified: t.trustUnverified,
              }}
              cardLabels={{
                promoted: t.promotedLabel,
                originAgent: t.originAgent,
                hasNotes: t.hasNotes,
                hasRelations: t.hasRelations,
                question: t.questionEyebrow,
                folded: t.foldedSummary,
                trustConfirmed: t.trustConfirmed,
                trustMachine: t.trustMachine,
                trustUnverified: t.trustUnverified,
                tests: t.nodeTests,
                testsFailing: t.nodeTestsFailing,
              }}
              nodeLabels={{
                heading: t.nodeDialogTitle,
                subtitle: t.nodeDialogSubtitle,
                origin: t.origin,
                originHuman: t.originHuman,
                originAgent: t.originAgent,
                promoted: t.promotedLabel,
                attachments: t.attachmentsBadge,
                noAttachments: t.attachmentsEmpty,
                openAttachments: t.openAttachments,
                notes: t.notes,
                notesHint: t.notesHint,
                notesCount: t.notesCount,
                kind: t.kind,
                shape: t.shape,
                color: t.color,
                colorNone: t.colorNone,
                edgeLabel: t.edgeLabel,
                edgeLabelHint: t.edgeLabelHint,
                reviewed: t.reviewed,
                relations: t.relations,
                removeRelation: t.removeRelation,
                noRelations: t.noRelations,
                close: t.close,
                readOnly: t.readOnlyBanner,
                kinds: {
                  thought: t.kindThought,
                  question: t.kindQuestion,
                  decision: t.kindDecision,
                  screen: t.kindScreen,
                  component: t.kindComponent,
                },
                shapes: {
                  rounded: t.shapeRounded,
                  square: t.shapeSquare,
                  pill: t.shapePill,
                },
                question: t.questionEyebrow,
                answer: t.answer,
                answerHint: t.answerHint,
                answerAction: t.answerAction,
                answerAbout: t.answerAbout,
                answerAlone: t.answerAlone,
              }}
              attachmentLabels={{
                title: t.attachmentsTitle,
                subtitle: t.attachmentsSubtitle,
                empty: t.attachmentsEmpty,
                count: t.attachmentsCount,
                full: t.attachmentsFull,
                kind: t.attachmentKind,
                name: t.attachmentName,
                gist: t.attachmentGist,
                ref: t.attachmentRef,
                add: t.addAttachment,
                addOpen: t.attachSomething,
                edit: t.editAttachment,
                save: t.saveAttachment,
                remove: t.removeAttachment,
                cancel: t.cancel,
                close: t.close,
                readOnly: t.readOnlyBanner,
                kinds: {
                  pdf: t.attPdf,
                  code: t.attCode,
                  table: t.attTable,
                  diagram: t.attDiagram,
                  audio: t.attAudio,
                  link: t.attLink,
                },
              }}
              pruneLabels={{
                title: t.pruneTitle,
                body: t.pruneBody,
                bodyLeaf: t.pruneBodyLeaf,
                confirmTitle: t.pruneConfirmTitle,
                confirmBody: t.pruneConfirmBody,
                watching: t.pruneWatching,
                next: t.pruneNext,
                remove: t.pruneRemove,
                cancel: t.cancel,
              }}
              detachLabels={{
                title: t.detachTitle,
                body: t.detachBody,
                confirmTitle: t.detachConfirmTitle,
                confirmBody: t.detachConfirmBody,
                carries: t.detachCarries,
                watching: t.pruneWatching,
                next: t.detachNext,
                detach: t.detachAction,
                cancel: t.cancel,
              }}
              paletteLabels={{
                scopeNode: t.paletteScopeNode,
                scopeMap: t.paletteScopeMap,
                placeholder: t.palettePlaceholder,
                noMatch: t.paletteNoMatch,
                keys: t.paletteKeys,
              }}
              commandLabels={{
                'node.child': t.cmdChild,
                'node.sibling': t.cmdSibling,
                'node.rename': t.cmdRename,
                'node.open': t.cmdOpen,
                'node.relate': t.cmdRelate,
                'node.attach': t.cmdAttach,
                'node.ask': t.cmdAsk,
                'node.promoteEpic': t.cmdPromoteEpic,
                'node.promoteInitiative': t.cmdPromoteInitiative,
                'node.collapse': t.cmdCollapse,
                'node.expand': t.cmdExpand,
                'node.delete': t.cmdDelete,
                'map.plan': t.cmdPlan,
                'map.tests': t.cmdTests,
                'map.goto': t.cmdGoto,
                'map.fit': t.cmdFit,
                'map.trust': t.trustLensCmd,
                'map.tidy': t.cmdTidy,
                'map.rename': t.cmdRenameMap,
                'map.project': t.cmdProject,
                'map.delete': t.cmdDeleteMap,
              }}
              commandHints={{
                'node.open': t.cmdOpenHint,
                'node.relate': t.cmdRelateHint,
                'node.ask': t.cmdAskHint,
                'map.trust': t.trustLensHint,
                'node.promoteEpic': t.promoteEpicHint,
                'node.promoteInitiative': t.promoteIniHint,
                'node.delete': t.cmdDeleteHint,
                'map.plan': t.cmdPlanHint,
                'map.tests': t.cmdTestsHint,
                'map.goto': t.cmdGotoHint,
                'map.delete': t.cmdDeleteMapHint,
              }}
            />
          ) : (
            <div className="text-muted-foreground px-6 py-16 text-center text-[13px]">
              {t.connecting}
            </div>
          )
        ) : selectedProject ? (
          // The project is chosen and has no brainstorm. Offer to start it here
          // rather than anywhere else: this is where somebody is looking when
          // they find out there is nothing to open.
          <div className="text-muted-foreground px-6 py-16 text-center">
            <div className="text-foreground mb-1.5 text-[15px] font-[680]">{t.startHere}</div>
            <div className="mb-4 text-[13px]">{t.startHereHint}</div>
            {canWrite && <Button onClick={() => void newMap(selectedProject)}>+ {t.newMap}</Button>}
            <div className="mt-3 text-[12px]">{t.paletteHint}</div>
          </div>
        ) : (
          <div className="text-muted-foreground px-6 py-16 text-center">
            <div className="text-foreground mb-1.5 text-[15px] font-[680]">{t.noProjects}</div>
          </div>
        )}
      </main>

      {paletteOpen && !open && (
        <CommandPalette
          scope={
            projects.find((p) => p.id === selectedProject)?.name || selectedProject || t.mindmaps
          }
          scopeKind="map"
          items={emptyItems}
          query={paletteQuery}
          onQuery={setPaletteQuery}
          active={paletteActive}
          onActive={setPaletteActive}
          onRun={(id) => {
            if (paletteStage === 'project') {
              setPaletteOpen(false)
              chooseProject(id)
              return
            }
            if (id === 'map.project') {
              setPaletteStage('project')
              setPaletteQuery('')
              setPaletteActive(null)
              return
            }
            setPaletteOpen(false)
            if (id === 'map.new') void newMap(selectedProject)
          }}
          onClose={() => setPaletteOpen(false)}
          labels={{
            scopeNode: t.paletteScopeNode,
            scopeMap: t.paletteScopeMap,
            placeholder: paletteStage === 'project' ? t.projectPlaceholder : t.palettePlaceholder,
            noMatch: t.paletteNoMatch,
            keys: t.paletteKeys,
          }}
        />
      )}
    </>
  )
}
