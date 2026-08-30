// /mindmaps — brainstorming, before any of it is an idea, with everyone in the
// room at once.
//
// THE CANVAS IS THE PAGE. There is no rail and no side panel: a project holds
// exactly one brainstorm, so a list of maps was a list of one, and a list of
// projects was navigation competing with the thing being navigated. What is left
// in the header is what a SHARED DOCUMENT needs and nothing else — its title,
// whether this browser is connected, and who else is looking at it.
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
// no dirty state. The page mints the socket ticket and owns the map's row
// metadata; `Live` owns the document.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'

import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { Button } from '@/components/ui/button'
import { CommandPalette } from '@/components/mindmap/CommandPalette'
import { TokenGate } from '@/components/TokenGate'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { useToast } from '@/components/Toaster'
import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { isAuthError, loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { listProjects, whoami, type Project } from '@/lib/initiatives'
import { fuzzyRank, isTextEntry } from '@/lib/mindmap-commands'
import {
  createMindmap,
  deleteMindmap,
  listMindmaps,
  mintMindmapSession,
  patchMindmap,
  promoteNode,
  type Mindmap,
  type MindmapSession,
} from '@/lib/mindmaps'
import Live, { type ConnectionState } from './Live'
import { STR } from './strings'

const LS_LANG = 'takomo.lang'

export function App() {
  const navigate = useNavigate()
  const { toast } = useToast()

  const [token, setToken] = useState(() => loadToken())
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem(LS_LANG)))
  const [project, setProject] = useState(() => loadProject())
  const [gateError, setGateError] = useState('')

  const [actor, setActor] = useState('')
  const [scopes, setScopes] = useState<string[]>([])
  const [navCollapsed, setNavCollapsed] = useNavCollapsed()
  const [projects, setProjects] = useState<Project[]>([])

  const [maps, setMaps] = useState<Mindmap[]>([])
  // `#m=<id>` so a map is a link somebody can send, the same way `/board#t=`
  // and `/inbox#q=` already are.
  const [openId, setOpenId] = useState<string | null>(
    () => new URLSearchParams(window.location.hash.slice(1)).get('m'),
  )
  const [session, setSession] = useState<MindmapSession | null>(null)
  const [connection, setConnection] = useState<ConnectionState>('connecting')
  const [peers, setPeers] = useState<string[]>([])

  // ⌘K, for the one case `Live` cannot cover: a project with no brainstorm has
  // no document, so nothing below this component is mounted to host the palette
  // — and with the rail gone that would leave no way out of the project at all.
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [paletteStage, setPaletteStage] = useState<'commands' | 'project'>('commands')
  const [paletteQuery, setPaletteQuery] = useState('')
  const [paletteActive, setPaletteActive] = useState<string | null>(null)

  const t = useMemo(() => pick(STR, lang), [lang])
  const canWrite = scopes.includes('write')
  const open = maps.find((m) => m.id === openId) ?? null
  const selectedProject = project || open?.project || projects[0]?.id || ''

  const handleErr = useCallback(
    (e: unknown) => {
      const err = e as { message?: string }
      if (isAuthError(e)) {
        saveToken('')
        setToken('')
        setGateError('')
        return
      }
      toast(err?.message || t.requestFailed, 'err')
    },
    [toast, t],
  )

  // Not filtered by project: ⌘K's "switch project…" needs to know which projects
  // HAVE a brainstorm, and one map per project keeps that list the size of the
  // project list rather than the size of the fleet's thinking.
  const refreshList = useCallback(async () => {
    const page = await listMindmaps(token, { limit: 100 })
    setMaps(page.items)
    return page.items
  }, [token])

  useEffect(() => {
    if (!token) return
    let cancelled = false
    ;(async () => {
      try {
        const who = await whoami(token)
        if (cancelled) return
        const sc = who.scopes ?? []
        if (!sc.includes('read')) {
          saveToken('')
          setToken('')
          setGateError(t.gateNoRead)
          return
        }
        setActor(who.actor ?? '')
        setScopes(sc)
        setProjects(await listProjects(token).catch(() => []))
      } catch (e) {
        if (!cancelled) handleErr(e)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [token, handleErr, t])

  useEffect(() => {
    if (!token) return
    refreshList()
      .then((items) => {
        // The deep-linked map, else this project's, else the newest-touched one:
        // a page that opens on nothing makes you pick before you can look.
        setOpenId(
          (current) =>
            current ??
            items.find((m) => m.project === (project || ''))?.id ??
            items[0]?.id ??
            null,
        )
      })
      .catch(handleErr)
  }, [token, refreshList, handleErr, project])

  // Opening a map means minting a ticket for its socket. Done here rather than
  // inside the canvas so the canvas never has to know about tokens — it receives
  // a session and connects.
  useEffect(() => {
    if (!token || !openId) {
      setSession(null)
      return
    }
    window.history.replaceState(null, '', `#m=${encodeURIComponent(openId)}`)
    let cancelled = false
    setSession(null)
    setConnection('connecting')
    setPeers([])
    mintMindmapSession(token, openId)
      .then((s) => {
        if (!cancelled) setSession(s)
      })
      .catch((e) => {
        if (!cancelled) handleErr(e)
      })
    return () => {
      cancelled = true
    }
  }, [token, openId, handleErr])

  const onConnection = useCallback((s: ConnectionState) => setConnection(s), [])
  const onPeers = useCallback((names: string[]) => setPeers(names), [])
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
        const { mindmap } = await createMindmap(token, { project: target, title: title.trim() })
        await refreshList()
        setOpenId(mindmap.id)
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
        setOpenId(null)
        toast(t.mapDeleted, 'success')
      })
      .catch(handleErr)
  }, [open, t, token, refreshList, toast, handleErr])

  const chooseProject = useCallback((id: string) => {
    setProject(id)
    saveProject(id)
    setOpenId(null)
  }, [])

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

  if (!token) {
    return (
      <TokenGate
        title="takomo · mindmaps"
        subtitle={t.gateTokenSub}
        tokenLabel={t.gateLabel}
        openLabel={t.gateOpen}
        emptyMessage={t.tokenNeeded}
        error={gateError}
        onSubmit={(tk) => {
          saveToken(tk)
          setGateError('')
          setToken(tk)
        }}
      />
    )
  }

  const connectionLabel =
    connection === 'connected'
      ? t.connected
      : connection === 'connecting'
        ? t.connecting
        : t.disconnected

  return (
    <AppShell
      rail={{
        onNavigate: navigate,
        current: 'mindmaps',
        nav: {
          board: t.board,
          inbox: t.inbox,
          documents: t.documents,
          initiatives: t.initiatives,
          mindmaps: t.mindmaps,
          schedules: t.schedules,
          verification: t.verification,
          environments: t.environments,
        },
        // No project picker here, deliberately: on this surface the project is a
        // ⌘K command rather than permanent chrome, because a canvas has room for
        // exactly one thing and it is the map.
        labels: {
          expand: t.navExpand,
          collapse: t.navCollapse,
          signOut: t.signOut,
          account: t.navAccount,
          settings: t.settings,
        },
        collapsed: navCollapsed,
        onCollapsed: setNavCollapsed,
        actor,
        scopes,
        onSignOut: () => {
          saveToken('')
          setToken('')
        },
      }}
    >
      {/* The header of a shared document says three things and no more: what it
          is, whether this browser is connected, and who else is in it. */}
      <AppHeader
        title={open ? open.title : t.mindmaps}
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
      >
        {open && (
          <>
            {/* No save button and no dirty state: the honest status on a shared
                document is whether this browser is connected. */}
            <span
              className={
                'text-[11.5px] font-bold ' +
                (connection === 'connected'
                  ? 'text-emerald-600 dark:text-emerald-400'
                  : 'text-muted-foreground')
              }
            >
              ● {connectionLabel}
            </span>
            <span className="text-muted-foreground text-[11.5px]">
              {peers.length ? `${t.alsoHere}: ${peers.join(', ')}` : t.justYou}
            </span>
          </>
        )}
      </AppHeader>

      <main className="flex min-h-0 flex-1 flex-col">
        {open ? (
          session ? (
            <Live
              key={session.session}
              session={session}
              title={open.title}
              onConnection={onConnection}
              onPeers={onPeers}
              onError={onLiveError}
              projects={projects.map((p) => ({ id: p.id, name: p.name || p.id }))}
              currentProject={open.project}
              onProject={chooseProject}
              canManageMap={canWrite}
              onRenameMap={renameMap}
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
              }}
              outlineLabels={{
                addChild: t.addChild,
                addSibling: t.addSibling,
                empty: t.canvasEmptyHint,
                hasNotes: t.hasNotes,
                hasContext: t.hasContext,
              }}
              cardLabels={{
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
                origin: t.origin,
                originHuman: t.originHuman,
                originAgent: t.originAgent,
                attachments: t.attachments,
                attachmentsFull: t.attachmentsFull,
                attachmentKind: t.attachmentKind,
                attachmentName: t.attachmentName,
                attachmentGist: t.attachmentGist,
                attachmentRef: t.attachmentRef,
                addAttachment: t.addAttachment,
                removeAttachment: t.removeAttachment,
                relations: t.relations,
                removeRelation: t.removeRelation,
                noRelations: t.noRelations,
                promoted: t.promotedLabel,
                readOnly: t.readOnlyBanner,
                hasNotes: t.hasNotes,
                hasContext: t.hasContext,
                kindThought: t.kindThought,
                kindQuestion: t.kindQuestion,
                kindDecision: t.kindDecision,
                kindScreen: t.kindScreen,
                kindComponent: t.kindComponent,
                shapeRounded: t.shapeRounded,
                shapeSquare: t.shapeSquare,
                shapePill: t.shapePill,
                attPdf: t.attPdf,
                attCode: t.attCode,
                attTable: t.attTable,
                attDiagram: t.attDiagram,
                attAudio: t.attAudio,
                attLink: t.attLink,
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
                'node.notes': t.cmdNotes,
                'node.relate': t.cmdRelate,
                'node.attach': t.cmdAttach,
                'node.promoteEpic': t.cmdPromoteEpic,
                'node.promoteInitiative': t.cmdPromoteInitiative,
                'node.collapse': t.cmdCollapse,
                'node.expand': t.cmdExpand,
                'node.delete': t.cmdDelete,
                'map.goto': t.cmdGoto,
                'map.fit': t.cmdFit,
                'map.tidy': t.cmdTidy,
                'map.rename': t.cmdRenameMap,
                'map.project': t.cmdProject,
                'map.delete': t.cmdDeleteMap,
              }}
              commandHints={{
                'node.relate': t.cmdRelateHint,
                'node.promoteEpic': t.promoteEpicHint,
                'node.promoteInitiative': t.promoteIniHint,
                'node.delete': t.cmdDeleteHint,
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
            placeholder:
              paletteStage === 'project' ? t.projectPlaceholder : t.palettePlaceholder,
            noMatch: t.paletteNoMatch,
            keys: t.paletteKeys,
          }}
        />
      )}
    </AppShell>
  )
}
