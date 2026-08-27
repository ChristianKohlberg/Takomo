// /documents — prose humans and agents write at the same time.
//
// The surface `/initiatives` could not be. An initiative's document is *reduced*
// from an append-only entry log, "latest view per pane wins", which is
// last-write-wins merge dressed as an audit trail: revising a paragraph means
// appending a whole new copy of the pane, and whatever somebody else wrote in
// the meantime loses. This page is the other answer — one CRDT, every
// participant an ordinary peer, merges handled by the data structure rather than
// by a policy anybody has to remember.
//
// It sits BESIDE /initiatives rather than replacing it. Nothing here writes to an
// initiative and nothing there is disturbed; a document can name the initiative
// it was distilled from, which is what makes an eventual migration expressible.
//
// The editor itself is a lazy import and must stay one: Tiptap + ProseMirror +
// Yjs outweigh the rest of the app, and every other surface would pay for them
// on first paint.
import { lazy, Suspense, useCallback, useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'

import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { TokenGate } from '@/components/TokenGate'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { isAuthError, loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { whoami, listProjects, type Project } from '@/lib/initiatives'
import {
  archiveDocument,
  buildTree,
  createDocument,
  listDocuments,
  mintSession,
  unarchiveDocument,
  type Doc,
  type DocSession,
  type Folder,
} from '@/lib/documents'
import { STR } from './strings'
import type { ConnectionState } from './Editor'
import { Checkbox } from '@/components/ui/checkbox'

const Editor = lazy(() => import('./Editor'))

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

  const [docs, setDocs] = useState<Doc[]>([])
  const [showArchived, setShowArchived] = useState(false)
  const [selected, setSelected] = useState<string | null>(null)
  const [session, setSession] = useState<DocSession | null>(null)
  const [connection, setConnection] = useState<ConnectionState>('connecting')
  const [peers, setPeers] = useState<string[]>([])
  const [pending, setPending] = useState(0)
  const [agentEnabled, setAgentEnabled] = useState(false)
  const [creating, setCreating] = useState(false)
  const [draftTitle, setDraftTitle] = useState('')
  const [draftPath, setDraftPath] = useState('')

  const t = useMemo(() => pick(STR, lang), [lang])
  const canWrite = scopes.includes('write')
  const current = useMemo(() => docs.find((d) => d.id === selected) ?? null, [docs, selected])

  function signOut() {
    saveToken('')
    setToken('')
    setDocs([])
    setSelected(null)
    setSession(null)
  }

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

  const fetchAll = useCallback(async () => {
    if (!project) {
      setDocs([])
      return
    }
    const page = await listDocuments(token, project, showArchived)
    setDocs(page.items)
  }, [token, project, showArchived])

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
        setAgentEnabled(Boolean(who.features?.doc_agent))
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
    fetchAll().catch(handleErr)
  }, [token, fetchAll, handleErr])

  // Opening a document means minting a ticket for its socket. Done here rather
  // than inside the editor so the editor never has to know about tokens — it
  // receives a session and connects.
  useEffect(() => {
    if (!token || !selected) {
      setSession(null)
      return
    }
    let cancelled = false
    setSession(null)
    setConnection('connecting')
    setPeers([])
    setPending(0)
    mintSession(token, selected)
      .then((s) => {
        if (!cancelled) setSession(s)
      })
      .catch((e) => {
        if (!cancelled) handleErr(e)
      })
    return () => {
      cancelled = true
    }
  }, [token, selected, handleErr])

  const onConnection = useCallback((s: ConnectionState) => setConnection(s), [])
  const onPeers = useCallback((names: string[]) => setPeers(names), [])
  const onPending = useCallback((n: number) => setPending(n), [])

  async function onCreate() {
    if (!canWrite) {
      toast(t.needWrite, 'err')
      return
    }
    const title = draftTitle.trim()
    if (!title) return
    try {
      const doc = await createDocument(token, project, {
        title,
        path: draftPath.trim() || undefined,
      })
      setCreating(false)
      setDraftTitle('')
      setDraftPath('')
      await fetchAll()
      setSelected(doc.id)
    } catch (e) {
      handleErr(e)
    }
  }

  async function onArchiveToggle(doc: Doc) {
    if (!canWrite) {
      toast(t.needWrite, 'err')
      return
    }
    if (!doc.archived_at && !confirm(t.confirmArchive)) return
    try {
      if (doc.archived_at) await unarchiveDocument(token, doc.id)
      else {
        await archiveDocument(token, doc.id)
        if (selected === doc.id) setSelected(null)
      }
      await fetchAll()
    } catch (e) {
      handleErr(e)
    }
  }

  const tree = useMemo(() => buildTree(docs), [docs])

  if (!token) {
    return (
      <TokenGate
        title="takomo · documents"
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
    connection === 'connected' ? t.connected : connection === 'connecting' ? t.connecting : t.disconnected

  return (
    <AppShell
      rail={{
        onNavigate: navigate,
        current: 'documents',
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
          setSelected(null)
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
        onSignOut: signOut,
      }}
    >
      <AppHeader
        title={t.documents}
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
      >
        <Button
          onClick={() => {
            if (!canWrite) {
              toast(t.needWrite, 'err')
              return
            }
            setCreating(true)
          }}
        >
          + {t.newDocument}
        </Button>
        <Button
          variant="outline"
          size="icon"
          title={t.refresh}
          onClick={() => fetchAll().catch(handleErr)}
        >
          ↻
        </Button>
      </AppHeader>

      {/* One breakpoint, `md`, meaning phone or not: the tree stacks above the
          document on a phone and sits beside it everywhere else. */}
      <main className="flex min-h-0 flex-1 flex-col overflow-hidden md:flex-row">
        <aside className="border-b-border-soft flex max-h-[38vh] flex-none flex-col overflow-y-auto border-b px-3 py-3 md:max-h-none md:w-full md:max-w-72 md:border-r md:border-b-0">
          {!project ? (
            <p className="text-muted-foreground px-1 py-4 text-[13px]">{t.pickProject}</p>
          ) : docs.length === 0 ? (
            <div className="text-muted-foreground px-1 py-6 text-[13px]">
              <p>{t.empty}</p>
              <p className="mt-2 text-[12.5px] opacity-80">{t.emptyHint}</p>
            </div>
          ) : (
            <FolderView
              folder={tree}
              depth={0}
              selected={selected}
              onSelect={setSelected}
              onArchiveToggle={onArchiveToggle}
              archiveLabel={t.archive}
              unarchiveLabel={t.unarchive}
              archivedLabel={t.archived}
            />
          )}

          <label className="text-muted-foreground mt-3 flex items-center gap-2 px-1 text-[12.5px]">
            <Checkbox
              checked={showArchived}
              onCheckedChange={(e) => setShowArchived(e === true)}
            />
            {t.showArchived}
          </label>
        </aside>

        <section className="flex min-w-0 flex-1 flex-col overflow-hidden px-4 py-4 md:px-6">
          {creating && (
            <div className="border-border-soft mb-4 flex flex-col gap-2 rounded-md border p-3">
              <input
                className="border-border-soft rounded-md border px-2 py-1.5 text-sm"
                placeholder={t.newDocumentTitle}
                value={draftTitle}
                autoFocus
                onChange={(e) => setDraftTitle(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') void onCreate()
                  if (e.key === 'Escape') setCreating(false)
                }}
              />
              <input
                className="border-border-soft rounded-md border px-2 py-1.5 text-sm"
                placeholder={t.newDocumentFolder}
                value={draftPath}
                onChange={(e) => setDraftPath(e.target.value)}
              />
              <div className="flex gap-2">
                <Button onClick={() => void onCreate()}>{t.create}</Button>
                <Button variant="outline" onClick={() => setCreating(false)}>
                  {t.cancel}
                </Button>
              </div>
            </div>
          )}

          {!current ? (
            <p className="text-muted-foreground px-1 py-8 text-center text-[13.5px]">
              {t.selectPrompt}
            </p>
          ) : (
            <>
              <div className="mb-3 flex flex-wrap items-baseline gap-x-3 gap-y-1">
                <h2 className="min-w-0 text-lg font-[750] tracking-[-0.02em]">{current.title}</h2>
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
                <span className="text-muted-foreground text-[11.5px]">
                  {current.updates === 1
                    ? t.historyOne
                    : t.historyMany.replace('{n}', String(current.updates))}
                </span>
                {pending > 0 && (
                  <span className="rounded-sm bg-amber-100 px-1.5 py-0.5 text-[11px] font-bold text-amber-900 dark:bg-amber-950 dark:text-amber-200">
                    {pending === 1 ? t.waitingOne : t.waitingMany.replace('{n}', String(pending))}
                  </span>
                )}
              </div>

              {session && (
                <Suspense
                  fallback={<p className="text-muted-foreground text-[13px]">{t.connecting}</p>}
                >
                  <Editor
                    key={session.session}
                    session={session}
                    onConnection={onConnection}
                    onPeers={onPeers}
                    onPending={onPending}
                    token={token}
                    agentEnabled={agentEnabled}
                    onError={handleErr}
                    labels={{
                      placeholder: t.placeholder,
                      readOnly: t.readOnlyBanner,
                      askHint: t.askHint,
                      agentOff: t.agentOff,
                    }}
                    menuLabels={{
                      placeholder: t.menuPlaceholder,
                      runFreeText: t.menuFreeText,
                      scoped: t.menuScoped,
                      whole: t.menuWhole,
                      close: t.menuClose,
                      running: t.menuRunning,
                    }}
                    suggestionLabels={{
                      tighten: t.sTighten,
                      simpler: t.sSimpler,
                      asCommitment: t.sAsCommitment,
                      asCommitmentHint: t.sAsCommitmentHint,
                      findContradiction: t.sFindContradiction,
                      findContradictionHint: t.sFindContradictionHint,
                      asList: t.sAsList,
                      expandSection: t.sExpandSection,
                      splitSection: t.sSplitSection,
                      addMissingItems: t.sAddMissingItems,
                      whatIsMissing: t.sWhatIsMissing,
                      whatIsMissingHint: t.sWhatIsMissingHint,
                      contradictions: t.sContradictions,
                      summarize: t.sSummarize,
                      addHeadings: t.sAddHeadings,
                      addHeadingsHint: t.sAddHeadingsHint,
                    }}
                    proposalLabels={{
                      heading: t.proposalsHeading,
                      empty: t.proposalsEmpty,
                      pending: t.proposalPending,
                      accepted: t.proposalAccepted,
                      rejected: t.proposalRejected,
                      accept: t.proposalAccept,
                      reject: t.proposalReject,
                      by: t.proposalBy,
                      partial: t.proposalPartial,
                      opReplace: t.opReplace,
                      opInsert: t.opInsert,
                      opDelete: t.opDelete,
                      needWrite: t.proposalNeedWrite,
                    }}
                  />
                </Suspense>
              )}
            </>
          )}
        </section>
      </main>
    </AppShell>
  )
}

interface FolderViewProps {
  folder: Folder
  depth: number
  selected: string | null
  onSelect: (id: string) => void
  onArchiveToggle: (doc: Doc) => void
  archiveLabel: string
  unarchiveLabel: string
  archivedLabel: string
}

function FolderView({
  folder,
  depth,
  selected,
  onSelect,
  onArchiveToggle,
  archiveLabel,
  unarchiveLabel,
  archivedLabel,
}: FolderViewProps) {
  return (
    <div>
      {folder.name && (
        <p
          className="text-muted-foreground mt-2 px-1 text-[11.5px] font-bold tracking-wide uppercase"
          style={{ paddingLeft: `${depth * 10}px` }}
        >
          {folder.name}
        </p>
      )}
      {folder.docs.map((doc) => (
        <div
          key={doc.id}
          className="group flex items-center gap-1"
          style={{ paddingLeft: `${depth * 10}px` }}
        >
          <button
            type="button"
            onClick={() => onSelect(doc.id)}
            className={
              'min-w-0 grow truncate rounded-md px-2 py-1.5 text-left text-[13.5px] ' +
              (selected === doc.id ? 'bg-accent font-semibold' : 'hover:bg-accent/50')
            }
          >
            {doc.title}
            {doc.archived_at && (
              <span className="text-muted-foreground ml-1.5 text-[11px]">({archivedLabel})</span>
            )}
          </button>
          <button
            type="button"
            title={doc.archived_at ? unarchiveLabel : archiveLabel}
            onClick={() => onArchiveToggle(doc)}
            className="text-muted-foreground hover:text-foreground flex-none px-1 text-[12px] opacity-0 group-hover:opacity-100 focus:opacity-100"
          >
            {doc.archived_at ? '↩' : '×'}
          </button>
        </div>
      ))}
      {folder.children.map((child) => (
        <FolderView
          key={child.path}
          folder={child}
          depth={depth + 1}
          selected={selected}
          onSelect={onSelect}
          onArchiveToggle={onArchiveToggle}
          archiveLabel={archiveLabel}
          unarchiveLabel={unarchiveLabel}
          archivedLabel={archivedLabel}
        />
      ))}
    </div>
  )
}
