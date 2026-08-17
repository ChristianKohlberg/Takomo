// /mindmaps — brainstorming, before any of it is an idea.
//
// A rail of maps and one canvas. The canvas is where the thinking happens; the
// rail exists because a project accumulates brainstorms and the newest is almost
// never the one you want.
//
// The page's own job is the part the canvas cannot do: keeping local state ahead
// of the server so typing never waits on a round trip, and turning a selected node
// into the two things a person does with it — grow from it, or graduate it.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'

import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { Button } from '@/components/ui/button'
import { Canvas } from '@/components/mindmap/Canvas'
import { Outline } from '@/components/mindmap/Outline'
import { TokenGate } from '@/components/TokenGate'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { useToast } from '@/components/Toaster'
import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { isAuthError, loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { listProjects, whoami, type Project } from '@/lib/initiatives'
import { childrenOf, positionAfter, type Point } from '@/lib/mindmap-layout'
import {
  addNodes,
  createMindmap,
  deleteMindmap,
  deleteNode,
  getMindmap,
  listMindmaps,
  patchNode,
  promoteNode,
  type Mindmap,
  type MindmapNode,
} from '@/lib/mindmaps'
import { cn } from '@/lib/utils'
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
  const [nodes, setNodes] = useState<MindmapNode[]>([])
  const [selected, setSelected] = useState<string | null>(null)

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

  const refreshOpen = useCallback(
    async (id: string) => {
      const detail = await getMindmap(token, id)
      setNodes(detail.nodes)
      setMaps((current) =>
        current.map((m) => (m.id === detail.mindmap.id ? detail.mindmap : m)),
      )
    },
    [token],
  )

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

  useEffect(() => {
    if (!token || !openId) {
      setNodes([])
      return
    }
    window.history.replaceState(null, '', `#m=${encodeURIComponent(openId)}`)
    refreshOpen(openId).catch(handleErr)
  }, [token, openId, refreshOpen, handleErr])

  /**
   * Run a write with the tree already updated locally.
   *
   * Typing must never wait on a round trip — that is the whole point of the
   * surface — so the optimistic tree goes in first and the server's answer
   * replaces it. A failure refetches rather than trying to unpick the edit: the
   * server is the truth, and a half-reverted tree is worse than a redraw.
   */
  const write = useCallback(
    async (optimistic: MindmapNode[] | null, run: () => Promise<unknown>) => {
      if (!canWrite) {
        toast(t.needWrite, 'err')
        return
      }
      if (optimistic) setNodes(optimistic)
      try {
        await run()
      } catch (e) {
        handleErr(e)
      } finally {
        if (openId) await refreshOpen(openId).catch(handleErr)
      }
    },
    [canWrite, toast, t, handleErr, openId, refreshOpen],
  )

  const addAfter = (id: string) => {
    if (!openId) return
    const node = nodes.find((n) => n.id === id)
    if (!node) return
    const siblings = childrenOf(nodes).get(node.parent ?? null) ?? []
    const position = positionAfter(siblings, id)
    void write(null, async () => {
      const { nodes: made } = await addNodes(token, openId, [
        {
          parent: node.parent,
          text: t.newThought,
          ...(position !== null ? { position } : {}),
        },
      ])
      // Select what was just made, so the next Enter continues from it.
      setSelected(made[0]?.id ?? null)
    })
  }

  const addChild = (id: string) => {
    if (!openId) return
    void write(null, async () => {
      const { nodes: made } = await addNodes(token, openId, [
        { parent: id, text: t.newThought },
      ])
      setSelected(made[0]?.id ?? null)
    })
  }

  const addBranch = () => {
    if (!openId) return
    void write(null, async () => {
      const { nodes: made } = await addNodes(token, openId, [{ text: t.newThought }])
      setSelected(made[0]?.id ?? null)
    })
  }

  const setText = (id: string, text: string) => {
    if (!openId) return
    const trimmed = text.trim()
    const current = nodes.find((n) => n.id === id)
    if (!current || trimmed === current.text) return
    if (!trimmed) {
      // An emptied node is a deletion in every outliner, and typing over a
      // first-draft thought then clearing it is the commonest way to say
      // "actually, no".
      void write(
        nodes.filter((n) => n.id !== id),
        () => deleteNode(token, openId, id),
      )
      return
    }
    void write(
      nodes.map((n) => (n.id === id ? { ...n, text: trimmed } : n)),
      () => patchNode(token, openId, id, { text: trimmed }),
    )
  }

  const remove = (id: string) => {
    if (!openId) return
    if (!window.confirm(t.confirmPrune)) return
    setSelected(null)
    void write(null, () => deleteNode(token, openId, id))
  }

  const reparent = (id: string, parent: string) => {
    if (!openId) return
    void write(
      // Optimistic: the node moves and un-pins, because a dropped node is placed
      // by the layout under its new parent.
      nodes.map((n) => (n.id === id ? { ...n, parent, at: null } : n)),
      () => patchNode(token, openId, id, { parent, at: null }),
    )
  }

  const place = (id: string, at: Point) => {
    if (!openId) return
    void write(
      nodes.map((n) => (n.id === id ? { ...n, at } : n)),
      () => patchNode(token, openId, id, { at }),
    )
  }

  const tidy = () => {
    if (!openId) return
    const pinned = nodes.filter((n) => n.at != null)
    if (pinned.length === 0) return
    void write(
      nodes.map((n) => ({ ...n, at: null })),
      () => Promise.all(pinned.map((n) => patchNode(token, openId, n.id, { at: null }))),
    )
  }

  const promote = (target: 'epic' | 'initiative') => {
    if (!openId || !selected) return
    void write(null, async () => {
      const { created } = await promoteNode(token, openId, selected, target)
      toast(
        (target === 'epic' ? t.promotedEpic : t.promotedInitiative).replace(
          '{id}',
          created.id,
        ),
        'success',
      )
    })
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
      setSelected(null)
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

  const selectedNode = nodes.find((n) => n.id === selected) ?? null

  return (
    <AppShell
      rail={{
        onNavigate: navigate,
        current: 'mindmaps',
        nav: {
          board: t.board,
          inbox: t.inbox,
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
                onClick={() => {
                  setOpenId(m.id)
                  setSelected(null)
                }}
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
                <span className="text-foreground mr-1 text-[13.5px] font-[700]">
                  {open.title}
                </span>
                <Button variant="outline" size="sm" onClick={addBranch}>
                  + {t.branch}
                </Button>
                {/* Promotion is offered only with a node selected, because that is
                    the only time the question ("what does THIS become?") exists. */}
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!selectedNode || !!selectedNode.promoted}
                  title={selectedNode?.promoted ? t.alreadyPromoted : t.promoteEpicHint}
                  onClick={() => promote('epic')}
                >
                  {t.makeEpic}
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={!selectedNode || !!selectedNode.promoted}
                  title={selectedNode?.promoted ? t.alreadyPromoted : t.promoteIniHint}
                  onClick={() => promote('initiative')}
                >
                  {t.makeInitiative}
                </Button>
                <span className="grow" />
                <span className="text-muted-foreground hidden text-[11.5px] md:inline">
                  {t.keysHint}
                </span>
                <Button variant="ghost" size="sm" onClick={() => void removeMap(open.id)}>
                  {t.deleteMap}
                </Button>
              </div>

              {/* The canvas is the desktop surface; a phone gets the same tree as a
                  list, which is a better shape for the screen rather than a
                  consolation prize. */}
              <Canvas
                className="hidden md:flex"
                mindmap={open}
                nodes={nodes}
                selected={selected}
                onSelect={setSelected}
                onText={setText}
                onSibling={addAfter}
                onChild={addChild}
                onDelete={remove}
                onReparent={reparent}
                onPlace={place}
                onTidy={tidy}
                labels={{
                  empty: t.canvasEmpty,
                  emptyHint: t.canvasEmptyHint,
                  fit: t.fit,
                  tidy: t.tidy,
                  zoomIn: t.zoomIn,
                  zoomOut: t.zoomOut,
                  cannotDrop: t.cannotDrop,
                }}
              />
              <div className="min-h-0 flex-1 overflow-y-auto md:hidden">
                <Outline
                  nodes={nodes}
                  selected={selected}
                  onSelect={setSelected}
                  onChild={addChild}
                  onSibling={addAfter}
                  labels={{
                    addChild: t.addChild,
                    addSibling: t.addSibling,
                    empty: t.canvasEmptyHint,
                  }}
                />
              </div>
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
