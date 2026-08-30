// /mindmaps — brainstorming, before any of it is an idea, with everyone in the
// room at once.
//
// A rail of maps and one live canvas. The rail exists because a project
// accumulates brainstorms and the newest is almost never the one you want.
//
// What changed here is the state source, and it changed the page's job with it.
// The map used to be rows: every keystroke was a REST write, every write was
// followed by a refetch, and the page carried an optimistic tree so typing did
// not wait on a round trip. It is now a CRDT — one replica shared by every
// browser and every agent — so there is no optimistic copy to keep, no refetch,
// no save button and no dirty state. The page mints the socket ticket and owns
// the map list; `Live` owns the document.
//
// The list itself is still ordinary REST, because a map's title and status are
// row metadata and were never the contended part.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'

import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { Button } from '@/components/ui/button'
import { TokenGate } from '@/components/TokenGate'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { useToast } from '@/components/Toaster'
import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { isAuthError, loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { listProjects, whoami, type Project } from '@/lib/initiatives'
import type { MapNode } from '@/lib/mindmap-doc'
import {
  createMindmap,
  deleteMindmap,
  listMindmaps,
  mintMindmapSession,
  promoteNode,
  type Mindmap,
  type MindmapSession,
} from '@/lib/mindmaps'
import { cn } from '@/lib/utils'
import Live, { type ConnectionState } from './Live'
import { STR } from './strings'
import { Hint } from '@/components/Hint'

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
  const [selectedNode, setSelectedNode] = useState<MapNode | null>(null)

  const t = useMemo(() => pick(STR, lang), [lang])
  const canWrite = scopes.includes('write')
  const open = maps.find((m) => m.id === openId) ?? null

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

  const refreshList = useCallback(async () => {
    const page = await listMindmaps(token, { project: project || undefined, limit: 100 })
    setMaps(page.items)
    return page.items
  }, [token, project])

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
        // Open the deep-linked map, else the newest-touched one: a page that opens
        // on nothing makes you pick before you can look.
        setOpenId((current) => current ?? items[0]?.id ?? null)
      })
      .catch(handleErr)
  }, [token, refreshList, handleErr])

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
    setSelectedNode(null)
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
  const onSelected = useCallback((node: MapNode | null) => setSelectedNode(node), [])
  const onLiveError = useCallback((message: string) => toast(message, 'err'), [toast])

  const promote = (target: 'epic' | 'initiative') => {
    if (!openId || !selectedNode) return
    // Promotion goes over REST on purpose: it creates an epic or an initiative,
    // which is work in the store rather than a change to this document. The
    // server writes the node's link into the same room, so it arrives here over
    // the socket like any other edit.
    promoteNode(token, openId, selectedNode.id, target)
      .then(({ created }) => {
        toast(
          (target === 'epic' ? t.promotedEpic : t.promotedInitiative).replace('{id}', created.id),
          'success',
        )
      })
      .catch(handleErr)
  }

  const newMap = async () => {
    if (!canWrite) {
      toast(t.needWrite, 'err')
      return
    }
    const title = window.prompt(t.newMapPrompt)
    if (!title?.trim()) return
    try {
      const { mindmap } = await createMindmap(token, {
        project: project || projects[0]?.id || '',
        title: title.trim(),
      })
      await refreshList()
      setOpenId(mindmap.id)
    } catch (e) {
      handleErr(e)
    }
  }

  const removeMap = async (id: string) => {
    if (!window.confirm(t.confirmDeleteMap)) return
    try {
      await deleteMindmap(token, id)
      const items = await refreshList()
      setOpenId(items[0]?.id ?? null)
      toast(t.mapDeleted, 'success')
    } catch (e) {
      handleErr(e)
    }
  }

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
        projects: projects.map(({ id, name, archived, archived_at }) => ({
          id,
          name,
          archived,
          archived_at,
        })),
        project,
        onProject: (id) => {
          setProject(id)
          saveProject(id)
          setOpenId(null)
        },
        projectLabels: {
          project: t.project,
          search: t.projectSearch,
          noMatch: t.projectNoMatch,
          all: t.allProjects,
        },
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
      <AppHeader
        title={t.mindmaps}
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
      >
        <Button onClick={() => void newMap()}>+ {t.newMap}</Button>
      </AppHeader>

      {/* Stacked on a phone, rail + canvas from `md` up. */}
      <div className="flex min-h-0 flex-1 flex-col md:flex-row">
        <aside className="border-b-border-soft md:border-r-border-soft flex max-h-40 shrink-0 flex-col overflow-y-auto border-b md:max-h-none md:w-60 md:border-r md:border-b-0">
          {maps.length === 0 ? (
            <div className="text-muted-foreground px-4 py-6 text-center text-[12.5px]">
              {t.noMaps}
            </div>
          ) : (
            maps.map((m) => (
              <button
                key={m.id}
                type="button"
                onClick={() => setOpenId(m.id)}
                aria-current={m.id === openId}
                className={cn(
                  'border-b-border-soft cursor-pointer border-b px-4 py-2.5 text-left',
                  m.id === openId && 'bg-accent',
                )}
              >
                <div className="text-foreground truncate text-[13px] font-[680]">{m.title}</div>
                <div className="text-muted-foreground font-mono text-[11px]">
                  {m.nodes} · {m.status}
                </div>
              </button>
            ))
          )}
        </aside>

        <main className="flex min-h-0 flex-1 flex-col">
          {open ? (
            <>
              <div className="border-b-border-soft flex flex-wrap items-center gap-2 border-b px-4 py-2">
                <span className="text-foreground mr-1 text-[13.5px] font-[700]">{open.title}</span>
                {/* No save button and no dirty state: the honest status on a
                    shared document is whether this browser is connected. */}
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
                <span className="grow" />
                {/* Promotion is offered only with a node selected, because that is
                    the only time the question ("what does THIS become?") exists. */}
                <Hint text={selectedNode?.promoted ? t.alreadyPromoted : t.promoteEpicHint}>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={!canWrite || !selectedNode || !!selectedNode.promoted}
                    onClick={() => promote('epic')}
                  >
                    {t.makeEpic}
                  </Button>
                </Hint>
                <Hint text={selectedNode?.promoted ? t.alreadyPromoted : t.promoteIniHint}>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={!canWrite || !selectedNode || !!selectedNode.promoted}
                    onClick={() => promote('initiative')}
                  >
                    {t.makeInitiative}
                  </Button>
                </Hint>
                <span className="text-muted-foreground hidden text-[11.5px] md:inline">
                  {t.keysHint}
                </span>
                <Button variant="ghost" size="sm" onClick={() => void removeMap(open.id)}>
                  {t.deleteMap}
                </Button>
              </div>

              {session ? (
                <Live
                  key={session.session}
                  session={session}
                  title={open.title}
                  onConnection={onConnection}
                  onPeers={onPeers}
                  onSelected={onSelected}
                  onError={onLiveError}
                  labels={{
                    branch: t.branch,
                    readOnly: t.readOnlyBanner,
                    newThought: t.newThought,
                    relationLabelPrompt: t.relationLabelPrompt,
                    capNodes: t.capNodes,
                    capRelationships: t.capRelationships,
                    needWrite: t.needWrite,
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
                  }}
                  detailsLabels={{
                    heading: t.detailsHeading,
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
                    attachmentsHint: t.attachmentsHint,
                    attachmentsFull: t.attachmentsFull,
                    attachmentKind: t.attachmentKind,
                    attachmentName: t.attachmentName,
                    attachmentGist: t.attachmentGist,
                    attachmentRef: t.attachmentRef,
                    addAttachment: t.addAttachment,
                    removeAttachment: t.removeAttachment,
                    relations: t.relations,
                    relationsHint: t.relationsHint,
                    startRelation: t.startRelation,
                    cancelRelation: t.cancelRelation,
                    removeRelation: t.removeRelation,
                    noRelations: t.noRelations,
                    promoted: t.promotedLabel,
                    deleteNode: t.deleteNode,
                    readOnly: t.readOnlyBanner,
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
                />
              ) : (
                <div className="text-muted-foreground px-6 py-16 text-center text-[13px]">
                  {t.connecting}
                </div>
              )}
            </>
          ) : (
            <div className="text-muted-foreground px-6 py-16 text-center">
              <div className="text-foreground mb-1.5 text-[15px] font-[680]">{t.noneOpen}</div>
              <div className="text-[13px]">{t.noneOpenHint}</div>
            </div>
          )}
        </main>
      </div>
    </AppShell>
  )
}
