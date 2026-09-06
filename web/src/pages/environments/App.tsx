// /environments — where a check can be run.
//
// Cards, not a table, and that is a mobile decision rather than a stylistic one:
// an environment carries two commands and a paragraph of caveats, which a table
// row cannot hold at 375px without either truncating the thing you came to read
// or scrolling sideways. The card stacks.
//
// What the page is FOR is the contrast between rows — a local box you can break,
// a staging tier that resets nightly, a production entry that is read-only. So
// `writable` and `data_state` are on the collapsed card, not behind a detail
// view: they are what an agent (or a person) checks before running anything
// destructive.
import { useCallback, useEffect, useMemo, useState } from 'react'
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
  archiveEnvironment,
  createEnvironment,
  listEnvironments,
  patchEnvironment,
  unarchiveEnvironment,
  type Environment,
  type EnvironmentFields,
} from '@/lib/verification'
import { EnvironmentCard } from '@/components/environments/EnvironmentCard'
import { EnvironmentDialog } from '@/components/environments/EnvironmentDialog'
import { STR } from './strings'
import { Checkbox } from '@/components/ui/checkbox'
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

  const [envs, setEnvs] = useState<Environment[]>([])
  const [showArchived, setShowArchived] = useState(false)
  const [editing, setEditing] = useState<Environment | null>(null)
  const [creating, setCreating] = useState(false)
  const [busy, setBusy] = useState<Record<string, boolean>>({})

  const t = useMemo(() => pick(STR, lang), [lang])
  const canWrite = scopes.includes('write')

  function signOut() {
    saveToken('')
    setToken('')
    setEnvs([])
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
      setEnvs([])
      return
    }
    const e = await listEnvironments(token, project, showArchived)
    setEnvs(e.items)
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

  const mutate = useCallback(
    async (id: string, run: () => Promise<unknown>) => {
      if (!canWrite) {
        toast(t.needWrite, 'err')
        return
      }
      setBusy((b) => ({ ...b, [id]: true }))
      try {
        await run()
        await fetchAll()
      } catch (e) {
        handleErr(e)
      } finally {
        setBusy((b) => ({ ...b, [id]: false }))
      }
    },
    [canWrite, toast, t, fetchAll, handleErr],
  )

  async function onCreate(fields: EnvironmentFields) {
    await createEnvironment(token, project, fields)
    await fetchAll()
  }

  async function onSave(id: string, fields: Record<string, unknown>) {
    await patchEnvironment(token, id, fields)
    await fetchAll()
  }

  if (!token) {
    return (
      <TokenGate
        title="takomo · environments"
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

  return (
    <AppShell
      lang={lang}
      onLang={(l) => { setLang(l); localStorage.setItem(LS_LANG, l) }}
      rail={{
        onNavigate: navigate,
        current: 'environments',
        nav: {
          board: t.board,
          inbox: t.inbox,
          specification: t.specification,
          initiatives: t.initiatives,
          schedules: t.schedules,
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
        title={t.environments}
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
          + {t.newEnvironment}
        </Button>
        <Hint text={t.refresh}>
          <Button
            variant="outline"
            size="icon"
            onClick={() => fetchAll().catch(handleErr)}
          >
            ↻
          </Button>
        </Hint>
      </AppHeader>

      <main className="min-h-0 flex-1 overflow-y-auto px-4 pt-4.5 pb-15 md:px-5">
        <div className="mx-auto flex w-full max-w-240 flex-col gap-3.5">
          <label className="text-muted-foreground flex items-center gap-2 self-start px-0.5 text-[12.5px]">
            <Checkbox
              checked={showArchived}
              onCheckedChange={(e) => setShowArchived(e === true)}
            />
            {t.showArchived}
          </label>

          {envs.length === 0 ? (
            <div className="text-muted-foreground px-1 py-7.5 text-center text-[13.5px]">
              <p>{t.empty}</p>
              <p className="mt-2 text-[12.5px] opacity-80">{t.emptyHint}</p>
            </div>
          ) : (
            envs.map((e) => (
              <EnvironmentCard
                key={e.id}
                env={e}
                busy={!!busy[e.id]}
                labels={{
                  archived: t.archived,
                  readOnly: t.readOnly,
                  writable: t.writable,
                  bringUp: t.bringUp,
                  teardown: t.teardown,
                  credentials: t.credentials,
                  credentialsNote: t.credentialsNote,
                  dataState: t.dataState,
                  notes: t.notes,
                  edit: t.edit,
                  archive: t.archive,
                  unarchive: t.unarchive,
                }}
                onEdit={() => {
                  if (!canWrite) {
                    toast(t.needWrite, 'err')
                    return
                  }
                  setEditing(e)
                }}
                onArchive={() => {
                  if (!window.confirm(t.confirmArchive)) return
                  void mutate(e.id, () => archiveEnvironment(token, e.id))
                }}
                onUnarchive={() => void mutate(e.id, () => unarchiveEnvironment(token, e.id))}
              />
            ))
          )}
        </div>
      </main>

      <EnvironmentDialog
        open={creating}
        onOpenChange={setCreating}
        existing={null}
        labels={t}
        onSubmit={onCreate}
      />
      <EnvironmentDialog
        open={!!editing}
        onOpenChange={(o) => !o && setEditing(null)}
        existing={editing}
        labels={t}
        onSubmit={async (fields) => {
          if (!editing) return
          // The slug is immutable — checks and tool calls address an environment
          // by it — so it never travels in a patch.
          const { slug: _slug, ...rest } = fields
          await onSave(editing.id, rest)
        }}
      />
    </AppShell>
  )
}
