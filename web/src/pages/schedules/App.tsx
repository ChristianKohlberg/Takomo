// /schedules — the recurrence page.
//
// Rows, not columns, and that is the design decision rather than a layout
// accident: the board sorts by state into columns, but a schedule's content is a
// HISTORY, so forcing cadences into columns would throw away the axis that
// carries all the meaning.
//
// Three groups in fixed order — waiting for you, active, stopped — because a
// proposal is the only row that is asking something of the reader.
import { useCallback, useEffect, useMemo, useState } from 'react'

import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { useNavigate } from 'react-router'
import { isAuthError, loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { TokenGate } from '@/components/TokenGate'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { CreateScheduleDialog } from '@/components/schedules/CreateScheduleDialog'
import { ScheduleCard } from '@/components/schedules/ScheduleCard'

import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { whoami, listProjects, type Project } from '@/lib/initiatives'
import {
  createSchedule,
  deleteSchedule,
  listWithHistory,
  runSchedule,
  scheduleAction,
  type Action,
  type CreateFields,
  type Schedule,
} from '@/lib/schedules'
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
  const [schedules, setSchedules] = useState<Schedule[]>([])
  const [busy, setBusy] = useState<Record<string, boolean>>({})
  const [creating, setCreating] = useState(false)

  const t = useMemo(() => pick(STR, lang), [lang])
  const isHuman = scopes.includes('human')

  function signOut() {
    saveToken('')
    setToken('')
    setSchedules([])
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
    setSchedules(await listWithHistory(token, project))
  }, [token, project])

  // Boot: identity first — a token with no `read` scope is refused at the gate
  // rather than left to fail on every request.
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

  // Every mutation goes through here: gate on `human`, mark the row busy so its
  // buttons cannot double-fire, then refetch — the row's history is what changed.
  const mutate = useCallback(
    async (id: string, run: () => Promise<unknown>, note?: string) => {
      if (!isHuman) {
        toast(t.needHuman, 'err')
        return
      }
      setBusy((b) => ({ ...b, [id]: true }))
      try {
        await run()
        if (note) toast(note, 'success')
        await fetchAll()
      } catch (e) {
        handleErr(e)
      } finally {
        setBusy((b) => {
          const next = { ...b }
          delete next[id]
          return next
        })
      }
    },
    [isHuman, toast, t, fetchAll, handleErr],
  )

  const onAction = (s: Schedule, action: Action) => {
    if (action === 'reject' && !window.confirm(t.confirmReject)) return
    void mutate(s.id, () => scheduleAction(token, s.id, action), t.saved)
  }

  const onRun = (s: Schedule) =>
    void mutate(s.id, async () => {
      const r = await runSchedule(token, s.id)
      // `run` is idempotent per slot: a slot that already produced a ticket
      // creates nothing, and saying so is more useful than a generic "saved".
      toast(r.created ? `${t.ranNew} ${r.ticket}` : t.ranNothing, 'success')
    })

  const onDelete = (s: Schedule) => {
    if (!window.confirm(t.confirmDelete)) return
    void mutate(s.id, () => deleteSchedule(token, s.id), t.deleted)
  }

  const onCreate = async (fields: CreateFields) => {
    const created = await createSchedule(token, fields)
    setCreating(false)
    toast(created.status === 'pending' ? t.pendingCreated : t.created, 'success')
    await fetchAll()
  }

  if (!token) {
    return (
      <TokenGate
        title="takomo · schedules"
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

  const pending = schedules.filter((s) => s.status === 'pending')
  const active = schedules.filter((s) => s.status === 'active')
  const stopped = schedules.filter((s) =>
    ['paused', 'rejected', 'retired'].includes(s.status),
  )

  const cardLabels = {
    every: t.everyN,
    onDay: t.onDay,
    day: t.unitDay,
    week: t.unitWeek,
    month: t.unitMonth,
    days: t.pluralDays,
    weeks: t.pluralWeeks,
    months: t.pluralMonths,
    statusPending: t.statusPending,
    statusActive: t.statusActive,
    statusPaused: t.statusPaused,
    statusRejected: t.statusRejected,
    statusRetired: t.statusRetired,
    actActivate: t.actActivate,
    actReject: t.actReject,
    actPause: t.actPause,
    actResume: t.actResume,
    actRun: t.actRun,
    actDelete: t.actDelete,
    outDone: t.outDone,
    outOpen: t.outOpen,
    outNf: t.outNf,
    nextAt: t.nextAt,
    noneScheduled: t.noneScheduled,
    proposedBy: t.proposedBy,
    lastN: t.lastN,
    nowArrow: t.nowArrow,
    neverFired: t.neverFired,
  }

  const openTicket = (ticket: string) => {
    // Client-side: the board reads `#t=` in a useState initializer, which runs
    // after the history entry is pushed, so the deep link still lands.
    navigate('/board#t=' + encodeURIComponent(ticket))
  }

  const renderGroup = (title: string, rows: Schedule[]) =>
    rows.length > 0 && (
      <div key={title} className="flex flex-col gap-3.5">
        <div className="text-muted-foreground flex items-center gap-2 px-0.5 pt-1.5 text-[11.5px] font-[750] tracking-[0.06em] uppercase">
          <span>{title}</span>
          <span className="text-muted-foreground font-semibold">{rows.length}</span>
        </div>
        {rows.map((s) => (
          <ScheduleCard
            key={s.id}
            schedule={s}
            labels={cardLabels}
            lang={lang}
            busy={!!busy[s.id]}
            onAction={(a) => onAction(s, a)}
            onRun={() => onRun(s)}
            onDelete={() => onDelete(s)}
            onOpenTicket={openTicket}
          />
        ))}
      </div>
    )

  return (
    <AppShell
      rail={{
        onNavigate: navigate,
        current: 'schedules',
        nav: {
          board: t.board,
          inbox: t.inbox,
          specification: t.specification,
          initiatives: t.initiatives,
          schedules: t.schedules,
          environments: t.environments,
        },
        badges: { schedules: pending.length },
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
        title={t.schedules}
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
      >
        <Button
          onClick={() => {
            if (!isHuman) {
              toast(t.needHuman, 'err')
              return
            }
            setCreating(true)
          }}
        >
          + {t.newSchedule}
        </Button>
        <Hint text={t.refresh}>
          <Button variant="outline" size="icon"onClick={() => fetchAll().catch(handleErr)}>
            ↻
          </Button>
        </Hint>
      </AppHeader>

      <main className="min-h-0 flex-1 overflow-y-auto px-5 pt-4.5 pb-15">
        <div className="mx-auto flex max-w-280 flex-col gap-3.5">
          {schedules.length === 0 ? (
            <div className="text-muted-foreground px-1 py-7.5 text-center text-[13.5px]">
              {t.emptyAll}
            </div>
          ) : (
            <>
              {pending.length > 0 && (
                <div className="bg-nfbg border-nfbd text-nf flex items-center gap-2.5 rounded-[10px] border px-3.5 py-2.5 text-[13px] font-[620]">
                  <span>●</span>
                  <span>{t.bannerPending}</span>
                </div>
              )}
              {renderGroup(t.groupPending, pending)}
              {renderGroup(t.groupActive, active)}
              {renderGroup(t.groupStopped, stopped)}
            </>
          )}
        </div>
      </main>

      <CreateScheduleDialog
        open={creating}
        onOpenChange={setCreating}
        project={project || projects[0]?.id || ''}
        onCreate={onCreate}
        labels={{
          title: t.newSchedule,
          subtitle: t.newSub,
          fName: t.fName,
          fNamePh: t.fNamePh,
          fEvery: t.fEvery,
          fInterval: t.fInterval,
          fDays: t.fDays,
          fDayOfMonth: t.fDayOfMonth,
          fAt: t.fAt,
          fTz: t.fTz,
          fTitle: t.fTitle,
          fTitlePh: t.fTitlePh,
          fTitleHint: t.fTitleHint,
          fBody: t.fBody,
          fLabels: t.fLabels,
          fLabelsPh: t.fLabelsPh,
          fRationale: t.fRationale,
          fRationalePh: t.fRationalePh,
          unitDay: t.unitDay,
          unitWeek: t.unitWeek,
          unitMonth: t.unitMonth,
          create: t.create,
          cancel: t.cancel,
          pickDay: t.pickDay,
        }}
      />
    </AppShell>
  )
}
