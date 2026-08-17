// /settings — the admin console.
//
// The page is new; almost nothing behind it is. Tokens and projects have had
// endpoints since long before this page existed, and an operator reached them
// through the CLI or curl. `GET /v1/export/sqlite` is the one thing that had to
// be built for it, because "download the database" was the only admin capability
// with no HTTP surface at all.
//
// Laid out as four SWITCHED sections rather than four stacked cards. Stacked,
// everything sat at one weight — "download the entire database, which contains
// every secret in this deployment" read exactly like "here is a list of project
// names" — and the page needed scrolling before it had any content. Switching
// gives each section the whole panel and makes the page's shape legible without
// scrolling it.
//
// The switcher is a TAB STRIP along the top, not a left sidebar. It was a
// sidebar until #132 gave the whole app a left nav rail, and two left rails side
// by side leave the reader unable to tell which one moves between surfaces and
// which one moves within this page.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useNavigate } from 'react-router'

import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { TokenGate } from '@/components/TokenGate'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { ConfirmDialog } from '@/components/settings/ConfirmDialog'
import { PeopleList } from '@/components/settings/PeopleList'
import { PersonDialog, type PersonSaved } from '@/components/settings/PersonDialog'
import {
  addMembership,
  createUser,
  listUsers,
  patchUser,
  removeMembership,
  setUserDisabled,
  type User,
} from '@/lib/users'
import { NewProjectDialog } from '@/components/settings/NewProjectDialog'
import { NewTokenDialog } from '@/components/settings/NewTokenDialog'
import { ProjectDetail } from '@/components/settings/ProjectDetail'
import { PromptDialog } from '@/components/settings/PromptDialog'
import { WorkflowEditor } from '@/components/settings/workflow/WorkflowEditor'
import { TokenList } from '@/components/settings/TokenList'
import { TokenRevealDialog } from '@/components/settings/TokenRevealDialog'
import {
  EmptyState,
  FactRow,
  Section,
  SectionTabs,
  type SectionDef,
} from '@/components/settings/SettingsShell'

import { isAuthError, loadToken, saveToken } from '@/lib/session'
import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { whoami, listProjects, type Project, type Whoami } from '@/lib/initiatives'
import {
  saveProjectSettings,
  settingsFrom,
  type ProjectSettings,
} from '@/lib/project-settings'
import {
  createWorkflowEntry,
  deleteWorkflowEntry,
  getProjectWorkflow,
  listWorkflows,
  patchWorkflowEntry,
  type Layout,
  type WorkflowDoc,
  type WorkflowEntry,
} from '@/lib/workflows'
import {
  archiveProject,
  createProject,
  createToken,
  deleteProject,
  downloadDatabase,
  formatBytes,
  listTokens,
  projectAllowlist,
  revokeToken,
  unarchiveProject,
  type CreatedToken,
  type TokenRow,
} from '@/lib/admin'
import { STR } from './strings'

const LS_LANG = 'takomo.lang'
const LS_SECTION = 'takomo.settings.section'

type SectionKey = 'overview' | 'data' | 'access' | 'people' | 'projects' | 'library'

/** `{name}`/`{size}`/`{id}`/`{actor}` substitution. */
function fill(template: string, values: Record<string, string>): string {
  return template.replace(/\{(\w+)\}/g, (m, k: string) => values[k] ?? m)
}

export function App() {
  const navigate = useNavigate()
  const { toast } = useToast()

  const [token, setToken] = useState(() => loadToken())
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem(LS_LANG)))
  const [gateError, setGateError] = useState('')
  // Remembered, because the reason someone opens /settings twice in a row is
  // usually the same reason — EXCEPT when a `?project=` deep link says otherwise.
  // The board's gear sends the reader here to configure one project, and honouring
  // the remembered tab over that landed them on Tokens with the project silently
  // selected behind it.
  const [section, setSection] = useState<SectionKey>(() => {
    if (new URLSearchParams(window.location.search).get('project')) return 'projects'
    const stored = localStorage.getItem(LS_SECTION)
    return stored === 'data' ||
      stored === 'access' ||
      stored === 'projects' ||
      stored === 'library'
      ? stored
      : 'overview'
  })

  const [who, setWho] = useState<Whoami | null>(null)
  const [projects, setProjects] = useState<Project[]>([])
  const [projectsLoadErr, setProjectsLoadErr] = useState('')
  const [tokens, setTokens] = useState<TokenRow[]>([])
  // The people directory. Beside the credentials because they answer the two
  // halves of one question: what may be done, and who work can be addressed to.
  const [people, setPeople] = useState<User[]>([])
  const [peopleBusy, setPeopleBusy] = useState('')
  // `null` = the dialog is closed. `{}` = adding somebody; a person = editing
  // them. One piece of state, so the two flows cannot both be open.
  const [editingPerson, setEditingPerson] = useState<User | 'new' | null>(null)

  // Which project's detail is open, and its editable settings. `?project=<id>`
  // is what the board's gear links to, so a deep link opens straight on the
  // project the reader was already looking at.
  const [selectedId, setSelectedId] = useState<string | null>(
    () => new URLSearchParams(window.location.search).get('project'),
  )
  const [settings, setSettings] = useState<ProjectSettings>(() => settingsFrom(undefined))
  const [origSettings, setOrigSettings] = useState<ProjectSettings>(() => settingsFrom(undefined))
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(false)
  const [saveErr, setSaveErr] = useState('')
  const [projectWorkflow, setProjectWorkflow] = useState<WorkflowDoc | null>(null)
  const [library, setLibrary] = useState<WorkflowEntry[]>([])

  const [navCollapsed, setNavCollapsed] = useNavCollapsed()
  const [exporting, setExporting] = useState(false)
  const [newToken, setNewToken] = useState(false)
  const [minted, setMinted] = useState<CreatedToken | null>(null)
  const [newProject, setNewProject] = useState(false)
  const [revoking, setRevoking] = useState<TokenRow | null>(null)
  const [deleting, setDeleting] = useState<Project | null>(null)
  // Two archive dialogs, not one with a flag: the second is a DIFFERENT question
  // (end someone's live lease), asked only after the server has refused the
  // first — so `force` is never the default and never silently attached.
  const [archiving, setArchiving] = useState<Project | null>(null)
  const [archivingForce, setArchivingForce] = useState<Project | null>(null)
  const [deletingWorkflow, setDeletingWorkflow] = useState<WorkflowEntry | null>(null)
  const [renaming, setRenaming] = useState<WorkflowEntry | null>(null)
  /** A draft waiting for a name before it goes into the library. */
  const [savingDraft, setSavingDraft] = useState<{ wf: WorkflowDoc; layout: Layout } | null>(null)

  const t = useMemo(() => pick(STR, lang), [lang])

  const isAdmin = (who?.scopes ?? []).includes('admin')
  // An allowlist is what makes the whole-database export unavailable, so the page
  // says why BEFORE the button is pressed rather than surfacing a 403 after.
  const allowlist = projectAllowlist(who)
  const scopedToProjects = allowlist !== null

  const selected = projects.find((p) => p.id === selectedId) ?? null

  /**
   * Open or close a project's detail, loading its saved values into the form.
   *
   * The URL is rewritten with `replaceState` rather than a router navigation:
   * the section is page state, not a route, and pushing a history entry per
   * project would make Back walk through them instead of leaving /settings.
   */
  const selectProject = useCallback((id: string | null) => {
    setSelectedId(id)
    setSaved(false)
    setSaveErr('')
    const url = new URL(window.location.href)
    if (id) url.searchParams.set('project', id)
    else url.searchParams.delete('project')
    window.history.replaceState(null, '', url)
  }, [])

  // Load a project's saved values into the form when the OPEN project changes.
  //
  // The ref guard is what makes this safe rather than the dependency list.
  // `refresh()` replaces the whole projects array after every write, so an
  // effect that reacted to `projects` would re-run and silently discard whatever
  // the reader had typed since. Reacting only when the open project's ID changes
  // also covers the `?project=` deep link, where the selection exists before the
  // fetch that resolves it.
  const selectedKey = selected?.id ?? ''
  const loadedFor = useRef<string | null>(null)
  useEffect(() => {
    if (loadedFor.current === selectedKey) return
    loadedFor.current = selectedKey
    const p = projects.find((x) => x.id === selectedKey)
    setSettings(settingsFrom(p))
    setOrigSettings(settingsFrom(p))
  }, [selectedKey, projects])

  // The open project's workflow and the shared library, for the editor below the
  // conventions. Both are fetched only when a project is actually open — the
  // list view needs neither.
  useEffect(() => {
    if (!selectedKey || !isAdmin) {
      setProjectWorkflow(null)
      return
    }
    let cancelled = false
    void getProjectWorkflow(token, selectedKey)
      .then((wf) => !cancelled && setProjectWorkflow(wf))
      .catch(() => !cancelled && setProjectWorkflow(null))
    return () => {
      cancelled = true
    }
  }, [selectedKey, isAdmin, token])

  // The library is needed by the editor's "Start from…" AND by its own section,
  // so it loads whenever either could be on screen.
  const reloadLibrary = useCallback(async () => {
    setLibrary(await listWorkflows(token).catch(() => [] as WorkflowEntry[]))
  }, [token])

  useEffect(() => {
    if (!isAdmin) return
    void reloadLibrary()
  }, [isAdmin, reloadLibrary])

  const onSaveProject = () => {
    if (!selected) return
    setSaving(true)
    setSaveErr('')
    saveProjectSettings(token, selected.id, settings, origSettings)
      .then(async (calls) => {
        // Nothing changed → say nothing. "Saved." over an untouched form is a
        // small lie that teaches the reader to distrust every later message.
        if (calls === 0) return
        setSaved(true)
        setOrigSettings(settings)
        await refresh()
      })
      .catch((e: Error) => setSaveErr(e.message))
      .finally(() => setSaving(false))
  }

  const SECTIONS: readonly SectionDef<SectionKey>[] = [
    { key: 'overview', label: t.navOverview, hint: t.navOverviewHint },
    { key: 'data', label: t.navData, hint: t.navDataHint },
    { key: 'access', label: t.navAccess, hint: t.navAccessHint },
    { key: 'people', label: t.navPeople, hint: t.navPeopleHint },
    { key: 'projects', label: t.navProjects, hint: t.navProjectsHint },
    { key: 'library', label: t.navLibrary, hint: t.navLibraryHint },
  ]

  /**
   * Save the dialog: the person, then their memberships.
   *
   * Membership is a diff rather than a set: the API has one route per edge (there
   * is no "replace all"), so this sends only what changed. Sequential on purpose —
   * one refused edge (an archived project freezes its memberships) should leave a
   * legible error rather than a half-applied fan-out whose failure order nobody
   * can reconstruct.
   */
  async function savePerson(fields: PersonSaved) {
    const existing = editingPerson !== 'new' && editingPerson ? editingPerson : null
    const handle = existing ? existing.handle : fields.handle
    if (existing) {
      await patchUser(token, handle, { name: fields.name, email: fields.email })
    } else {
      // The memberships picked at creation ride along in the same transaction, so
      // a new person and their first project land together or not at all.
      await createUser(token, {
        handle,
        name: fields.name,
        email: fields.email ?? undefined,
        projects: fields.projects,
      })
    }
    const before = new Set(existing ? (existing.projects ?? []) : fields.projects)
    const after = new Set(fields.projects)
    for (const p of after) {
      if (!before.has(p)) await addMembership(token, handle, p)
    }
    for (const p of before) {
      if (!after.has(p)) await removeMembership(token, handle, p)
    }
    setEditingPerson(null)
    await refresh()
  }

  function signOut() {
    saveToken('')
    setToken('')
    setWho(null)
    setTokens([])
    setProjects([])
  }

  const handleErr = useCallback(
    (e: unknown) => {
      if (isAuthError(e)) {
        saveToken('')
        setToken('')
        setGateError('')
        return
      }
      toast((e as { message?: string })?.message || t.requestFailed, 'err')
    },
    [toast, t],
  )

  /**
   * Archive a project, escalating to the force dialog when a worker holds a
   * lease.
   *
   * The 409 is not an error to report here — it is the server asking a question
   * this page can only answer by asking the reader. Anything else is a real
   * failure and goes to the toast.
   */
  const archive = useCallback(
    async (p: Project, force: boolean) => {
      try {
        await archiveProject(token, p.id, force)
        toast(fill(t.projArchivedToast, { id: p.id }), 'success')
        await refreshRef.current()
      } catch (e) {
        if (!force && (e as { code?: string })?.code === 'project.active_claims') {
          setArchivingForce(p)
          return
        }
        handleErr(e)
      }
    },
    [token, toast, t, handleErr],
  )

  const unarchive = useCallback(
    async (p: Project) => {
      try {
        await unarchiveProject(token, p.id)
        toast(fill(t.projUnarchivedToast, { id: p.id }), 'success')
        await refreshRef.current()
      } catch (e) {
        handleErr(e)
      }
    },
    [token, toast, t, handleErr],
  )

  const refresh = useCallback(async () => {
    let ps: Project[] = []
    try {
      ps = await listProjects(token)
      setProjectsLoadErr('')
    } catch (e) {
      setProjectsLoadErr((e as Error).message || t.requestFailed)
    }
    const ts = await listTokens(token).catch(() => [] as TokenRow[])
    // Disabled people included, deliberately: this is the surface where somebody
    // is put back, so hiding them here would hide the only control that undoes it.
    const us = await listUsers(token, { includeDisabled: true, limit: 200 })
      .then((page) => page.items)
      .catch(() => [] as User[])
    setProjects(ps)
    setTokens(ts)
    setPeople(us)
  }, [token, t])

  // `archive`/`unarchive` are declared above `refresh` (they read better next to
  // the dialogs they serve) and would otherwise capture it before it exists.
  const refreshRef = useRef<() => Promise<void>>(async () => {})
  refreshRef.current = refresh

  useEffect(() => {
    if (!token) return
    let cancelled = false
    ;(async () => {
      try {
        const w = await whoami(token)
        if (cancelled) return
        setWho(w)
        if ((w.scopes ?? []).includes('admin')) await refresh()
      } catch (e) {
        if (!cancelled) handleErr(e)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [token, refresh, handleErr])

  // A 403 from the export means "this token may not take a whole-database dump"
  // — a refusal of ONE operation by a token otherwise entitled to this page,
  // which `handleErr` now shows as a toast rather than a sign-out. This used to
  // need a hand-rolled bypass of `handleErr`; the rule lives in `isAuthError`.
  const onExport = async () => {
    setExporting(true)
    try {
      const { filename, bytes } = await downloadDatabase(token)
      toast(fill(t.dataDone, { name: filename, size: formatBytes(bytes) }), 'success')
    } catch (e) {
      handleErr(e)
    } finally {
      setExporting(false)
    }
  }

  if (!token) {
    return (
      <TokenGate
        title="takomo · settings"
        subtitle={t.gateTokenSub}
        tokenLabel={t.gateLabel}
        openLabel={t.gateOpen}
        emptyMessage={t.gateEmpty}
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
      rail={{
        onNavigate: navigate,
        current: 'account',
        nav: {
          board: t.board,
          inbox: t.inbox,
          initiatives: t.initiatives,
          schedules: t.schedules,
          verification: t.verification,
          environments: t.environments,
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
        actor: who?.actor,
        scopes: who?.scopes,
        onSignOut: signOut,
      }}
    >
      <AppHeader
        title={t.settings}
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
      />

      {/* max-w-3xl, not the wider column this used with a sidebar: the global
          nav rail now takes the left edge, so the panel starts further in and a
          5xl column pushed the content off-centre on a laptop. */}
      <div className="mx-auto flex w-full max-w-3xl flex-1 flex-col gap-5 overflow-y-auto px-5 py-6">
        {isAdmin && (
          <SectionTabs
            sections={SECTIONS}
            current={section}
            onSelect={(k) => {
              setSection(k)
              localStorage.setItem(LS_SECTION, k)
            }}
          />
        )}

        <div className="min-w-0 flex-1 pb-10">
          {!isAdmin ? (
            <Section title={t.notAdminTitle} description={t.notAdmin}>
              <code className="bg-muted block overflow-x-auto rounded-lg px-3 py-2.5 font-mono text-[12.5px] whitespace-pre">
                {t.notAdminCmd}
              </code>
            </Section>
          ) : section === 'overview' ? (
            <Section title={t.overviewTitle} description={t.overviewSub}>
              <dl className="border-border-soft bg-card rounded-xl border px-4">
                <FactRow label={t.factActor}>
                  <span className="font-mono">{who?.actor ?? '…'}</span>
                </FactRow>
                <FactRow label={t.factScopes}>
                  <span className="flex flex-wrap gap-1">
                    {(who?.scopes ?? []).map((s) => (
                      <Badge key={s} variant="secondary">
                        {s}
                      </Badge>
                    ))}
                  </span>
                </FactRow>
                <FactRow label={t.factProjects}>
                  {scopedToProjects ? (
                    <span className="flex flex-wrap gap-1">
                      {(allowlist ?? []).map((p) => (
                        <Badge key={p} variant="outline" className="font-mono">
                          {p}
                        </Badge>
                      ))}
                    </span>
                  ) : (
                    <span className="text-muted-foreground">{t.allProjects}</span>
                  )}
                </FactRow>
                <FactRow label={t.factTokenId}>
                  <span className="text-muted-foreground font-mono text-[12px]">
                    {who?.token_id ?? '…'}
                  </span>
                </FactRow>
              </dl>
            </Section>
          ) : section === 'data' ? (
            <Section title={t.dataTitle} description={t.dataSub}>
              <p className="text-muted-foreground max-w-prose text-[13px] leading-relaxed">
                {t.dataHow}
              </p>

              {scopedToProjects ? (
                <Callout tone="muted" title={t.dataScopedTitle}>
                  {t.dataScoped}
                </Callout>
              ) : (
                <>
                  <Callout tone="warn" title={t.dataWarnTitle}>
                    {t.dataWarn}
                  </Callout>
                  <div>
                    <Button onClick={() => void onExport()} disabled={exporting}>
                      {exporting ? t.dataBusy : t.dataBtn}
                    </Button>
                  </div>
                </>
              )}
            </Section>
          ) : section === 'access' ? (
            <Section
              title={t.accessTitle}
              description={t.accessSub}
              action={
                <Button onClick={() => setNewToken(true)}>+&nbsp;{t.accessNew}</Button>
              }
            >
              {tokens.length === 0 ? (
                <EmptyState>{t.accessEmpty}</EmptyState>
              ) : (
                <TokenList
                  tokens={tokens}
                  currentTokenId={who?.token_id}
                  labels={{
                    scopes: t.tokScopes,
                    projects: t.tokProjects,
                    allProjects: t.allProjects,
                    lastUsed: t.tokLastUsed,
                    neverUsed: t.tokNeverUsed,
                    revoked: t.tokRevoked,
                    expired: t.tokExpired,
                    revoke: t.tokRevoke,
                    thisToken: t.tokThisToken,
                  }}
                  onRevoke={setRevoking}
                />
              )}
            </Section>
          ) : section === 'people' ? (
            <Section
              title={t.peopleTitle}
              description={t.peopleSub}
              action={
                <Button onClick={() => setEditingPerson('new')}>+&nbsp;{t.peopleAdd}</Button>
              }
            >
              {people.length === 0 ? (
                <EmptyState>{t.peopleEmpty}</EmptyState>
              ) : (
                <PeopleList
                  people={people}
                  busyHandle={peopleBusy}
                  labels={{
                    person: t.peoplePerson,
                    projects: t.peopleProjects,
                    noProjects: t.peopleNoProjects,
                    status: t.peopleStatus,
                    active: t.peopleActive,
                    disabled: t.peopleDisabled,
                    edit: t.peopleEdit,
                    disable: t.peopleDisable,
                    enable: t.peopleEnable,
                    disableHint: t.peopleDisableHint,
                  }}
                  onEdit={(person) => setEditingPerson(person)}
                  onSetDisabled={(person, disabled) => {
                    setPeopleBusy(person.handle)
                    setUserDisabled(token, person.handle, disabled)
                      .then(() => refresh())
                      .catch(handleErr)
                      .finally(() => setPeopleBusy(''))
                  }}
                />
              )}
            </Section>
          ) : section === 'library' ? (
            <Section title={t.libTitle} description={t.libSub}>
              {library.length === 0 ? (
                <EmptyState>{t.libEmpty}</EmptyState>
              ) : (
                <ul className="flex flex-col gap-px">
                  {library.map((w) => (
                    <li
                      key={w.id}
                      className="bg-card border-border-soft flex flex-wrap items-center gap-x-3 gap-y-1.5 border px-3.5 py-3 first:rounded-t-xl last:rounded-b-xl [&:not(:first-child)]:border-t-0"
                    >
                      <div className="min-w-0 flex-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <span className="text-[13px] font-[650]">{w.name}</span>
                          {w.builtin && (
                            <Badge variant="secondary" title={t.libBuiltinLocked}>
                              {t.libBuiltin}
                            </Badge>
                          )}
                        </div>
                        <div className="text-muted-foreground mt-0.5 text-[11.5px]">
                          {t.libStates.replace('{n}', String(w.workflow.states.length))}
                          {' · '}
                          {t.libTransitions.replace(
                            '{n}',
                            String(w.workflow.transitions?.length ?? 0),
                          )}
                          {w.description ? ` · ${w.description}` : ''}
                        </div>
                      </div>
                      {/* Built-ins carry no actions at all rather than disabled
                          ones: they are reseeded on every start, so every edit
                          here would be undone silently. The badge's title says
                          so, and the row stays readable. */}
                      {!w.builtin && (
                        <>
                          <Button
                            variant="ghost"
                            size="sm"
                            onClick={() => setRenaming(w)}
                          >
                            {t.libRename}
                          </Button>
                          <Button
                            variant="destructive"
                            size="sm"
                            onClick={() => setDeletingWorkflow(w)}
                          >
                            {t.libDelete}
                          </Button>
                        </>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </Section>
          ) : projectsLoadErr && selectedId && !selected ? (
            <Section title={t.projTitle} description={t.projLoadErr}>
              <p className="text-destructive text-[13px]">{projectsLoadErr}</p>
              <Button variant="secondary" size="sm" onClick={() => selectProject(null)}>
                {t.projBack}
              </Button>
            </Section>
          ) : selected ? (
            <ProjectDetail
              project={selected}
              workflowSlot={
                projectWorkflow && (
                  <WorkflowEditor
                    token={token}
                    project={selected.id}
                    workflow={projectWorkflow}
                    library={library}
                    readOnly={!isAdmin}
                    onApplied={(wf) => {
                      setProjectWorkflow(wf)
                      void refresh()
                    }}
                    onError={handleErr}
                    onSaveAs={(wf, layout) => setSavingDraft({ wf, layout })}
                    labels={{
                      title: t.wfTitle,
                      subtitle: t.wfSubtitle,
                      addState: t.wfAddState,
                      startFrom: t.wfStartFrom,
                      apply: t.wfApply,
                      applying: t.wfApplying,
                      applied: t.wfApplied,
                      revert: t.wfRevert,
                      saveAs: t.wfSaveAs,
                      problems: t.wfProblems,
                      valid: t.wfValid,
                      checking: t.wfChecking,
                      blockedTitle: t.wfBlockedTitle,
                      blockedBody: t.wfBlockedBody,
                      blockedRow: t.wfBlockedRow,
                      openBoard: t.wfOpenBoard,
                      canvasInitial: t.wfCanvasInitial,
                      canvasClaimable: t.wfCanvasClaimable,
                      canvasTerminal: t.wfCanvasTerminal,
                      canvasHint: t.wfCanvasHint,
                      readOnlyMsg: t.wfReadOnly,
                      newStateId: t.wfNewStateId,
                      nothing: t.wfNothing,
                      stateTitle: t.wfStateTitle,
                      transitionTitle: t.wfTransitionTitle,
                      id: t.wfId,
                      idHint: t.wfIdHint,
                      category: t.wfCategory,
                      claimable: t.wfClaimable,
                      claimableHint: t.wfClaimableHint,
                      terminal: t.wfTerminal,
                      terminalHint: t.wfTerminalHint,
                      makeInitial: t.wfMakeInitial,
                      isInitial: t.wfIsInitial,
                      deleteState: t.wfDeleteState,
                      deleteTransition: t.wfDeleteTransition,
                      requires: t.wfRequires,
                      reqClaim: t.wfReqClaim,
                      reqHuman: t.wfReqHuman,
                      reqNoChildren: t.wfReqNoChildren,
                      reqNoBlockers: t.wfReqNoBlockers,
                      reqHasLink: t.wfReqHasLink,
                      reqHasLinkHint: t.wfReqHasLinkHint,
                      linkKey: t.wfLinkKey,
                      from: t.wfFrom,
                      to: t.wfTo,
                    }}
                  />
                )
              }
              settings={settings}
              onChange={(patch) => {
                setSettings((cur) => ({ ...cur, ...patch }))
                setSaved(false)
              }}
              readOnly={!isAdmin}
              saving={saving}
              saved={saved}
              error={saveErr}
              onSave={onSaveProject}
              onBack={() => selectProject(null)}
              onDelete={() => setDeleting(selected)}
              onToggleArchive={() => {
                if (selected.archived) void unarchive(selected)
                else setArchiving(selected)
              }}
              labels={{
                back: t.projBack,
                workflowLabel: t.projWorkflowLabel,
                langLabel: t.projLangLabel,
                langHelp: t.projLangHelp,
                langPh: t.projLangPh,
                styleLabel: t.projStyleLabel,
                styleHelp: t.projStyleHelp,
                stylePh: t.projStylePh,
                chars: t.projChars,
                ttlLabel: t.projTtlLabel,
                ttlHelp: t.projTtlHelp,
                claimTtlLabel: t.projClaimTtlLabel,
                claimTtlHelp: t.projClaimTtlHelp,
                maxClaimTtlLabel: t.projMaxClaimTtlLabel,
                maxClaimTtlHelp: t.projMaxClaimTtlHelp,
                save: t.projSave,
                saving: t.projSaving,
                savedMsg: t.projSaved,
                readOnlyMsg: t.projReadOnly,
                archived: t.projArchived,
                archivedBanner: t.projArchivedBanner,
                archive: t.projArchive,
                unarchive: t.projUnarchive,
                over: t.projOver,
                delete: t.projDelete,
              }}
            />
          ) : (
            <Section
              title={t.projTitle}
              description={t.projSub}
              action={<Button onClick={() => setNewProject(true)}>+&nbsp;{t.projNew}</Button>}
            >
              {projectsLoadErr ? (
                <EmptyState>{projectsLoadErr}</EmptyState>
              ) : projects.length === 0 ? (
                <EmptyState>{t.projEmpty}</EmptyState>
              ) : (
                <ul className="flex flex-col gap-px">
                  {projects.map((p) => (
                    <li
                      key={p.id}
                      className="bg-card border-border-soft flex flex-wrap items-center gap-x-3 gap-y-1.5 border px-3.5 py-3 first:rounded-t-xl last:rounded-b-xl [&:not(:first-child)]:border-t-0"
                    >
                      <span className="font-mono text-[13px] font-[650]">{p.id}</span>
                      <span className="text-muted-foreground min-w-0 flex-1 truncate text-[13px]">
                        {p.name ?? ''}
                      </span>
                      {p.workflow && <Badge variant="outline">{p.workflow}</Badge>}
                      {p.archived && <Badge variant="secondary">{t.projArchived}</Badge>}
                      <Button variant="secondary" size="sm" onClick={() => selectProject(p.id)}>
                        {t.projOpen}
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </Section>
          )}
        </div>
      </div>

      <PersonDialog
        open={editingPerson !== null}
        onOpenChange={(o) => {
          if (!o) setEditingPerson(null)
        }}
        person={editingPerson === 'new' ? null : editingPerson}
        projects={projects}
        labels={{
          addTitle: t.personAddTitle,
          addSubtitle: t.personAddSub,
          editTitle: t.personEditTitle,
          editSubtitle: t.personEditSub,
          handle: t.personHandle,
          handlePh: t.personHandlePh,
          handleHint: t.personHandleHint,
          handleFixed: t.personHandleFixed,
          name: t.personName,
          namePh: t.personNamePh,
          nameHint: t.personNameHint,
          email: t.personEmail,
          emailPh: t.personEmailPh,
          emailHint: t.personEmailHint,
          projects: t.personProjects,
          projectsHint: t.personProjectsHint,
          noProjectsPicked: t.personNoProjectsPicked,
          save: t.personSave,
          add: t.personAdd,
          cancel: t.cancel,
          needHandle: t.personNeedHandle,
          badHandle: t.personBadHandle,
        }}
        onSave={savePerson}
      />

      <NewTokenDialog
        open={newToken}
        onOpenChange={setNewToken}
        projects={projects}
        labels={{
          title: t.newTokTitle,
          subtitle: t.newTokSub,
          actor: t.newTokActor,
          actorPh: t.newTokActorPh,
          actorHint: t.newTokActorHint,
          scopes: t.newTokScopes,
          scopeRead: t.newTokRead,
          scopeWrite: t.newTokWrite,
          scopeHuman: t.newTokHuman,
          scopeAdmin: t.newTokAdmin,
          projects: t.newTokProjects,
          projectsHint: t.newTokProjectsHint,
          allProjects: t.newTokAll,
          create: t.newTokCreate,
          cancel: t.newTokCancel,
          needActor: t.newTokNeedActor,
          needScope: t.newTokNeedScope,
        }}
        onCreate={async (fields) => {
          const created = await createToken(token, fields)
          setNewToken(false)
          setMinted(created)
          await refresh()
        }}
      />

      <TokenRevealDialog
        token={minted?.token ?? null}
        actor={minted?.actor}
        labels={{
          title: t.revealTitle,
          subtitle: t.revealSub,
          copy: t.revealCopy,
          copied: t.revealCopied,
          done: t.revealDone,
        }}
        onClose={() => setMinted(null)}
      />

      <NewProjectDialog
        open={newProject}
        onOpenChange={setNewProject}
        labels={{
          title: t.newProjTitle,
          subtitle: t.newProjSub,
          id: t.newProjId,
          idPh: t.newProjIdPh,
          idHint: t.newProjIdHint,
          idInvalid: t.newProjIdInvalid,
          name: t.newProjName,
          namePh: t.newProjNamePh,
          nameHint: t.newProjNameHint,
          create: t.newProjCreate,
          cancel: t.newProjCancel,
        }}
        onCreate={async (fields) => {
          await createProject(token, fields)
          setNewProject(false)
          await refresh()
        }}
      />

      <ConfirmDialog
        open={revoking !== null}
        onOpenChange={(o) => !o && setRevoking(null)}
        title={t.confirmRevokeTitle}
        description={fill(t.confirmRevokeBody, { actor: revoking?.actor ?? '' })}
        confirmLabel={t.confirmRevokeYes}
        cancelLabel={t.cancel}
        onConfirm={async () => {
          if (!revoking) return
          try {
            await revokeToken(token, revoking.id)
            await refresh()
          } catch (e) {
            handleErr(e)
          }
        }}
      />

      <ConfirmDialog
        open={deleting !== null}
        onOpenChange={(o) => !o && setDeleting(null)}
        title={t.confirmDeleteProjTitle}
        description={fill(t.confirmDeleteProjBody, { id: deleting?.id ?? '' })}
        confirmLabel={t.confirmDeleteProjYes}
        cancelLabel={t.cancel}
        onConfirm={async () => {
          if (!deleting) return
          try {
            await deleteProject(token, deleting.id)
            // Deleting the project whose detail is open would otherwise leave
            // the panel showing a project that no longer exists.
            if (deleting.id === selectedId) selectProject(null)
            await refresh()
          } catch (e) {
            handleErr(e)
          }
        }}
      />
      <ConfirmDialog
        open={archiving !== null}
        onOpenChange={(o) => !o && setArchiving(null)}
        title={t.confirmArchiveTitle}
        description={fill(t.confirmArchiveBody, { id: archiving?.id ?? '' })}
        confirmLabel={t.confirmArchiveYes}
        cancelLabel={t.cancel}
        onConfirm={async () => {
          if (archiving) await archive(archiving, false)
        }}
      />
      <ConfirmDialog
        open={archivingForce !== null}
        onOpenChange={(o) => !o && setArchivingForce(null)}
        title={t.confirmArchiveForceTitle}
        description={fill(t.confirmArchiveForceBody, { id: archivingForce?.id ?? '' })}
        confirmLabel={t.confirmArchiveForceYes}
        cancelLabel={t.cancel}
        onConfirm={async () => {
          if (archivingForce) await archive(archivingForce, true)
        }}
      />
      <PromptDialog
        open={savingDraft !== null}
        onOpenChange={(o) => !o && setSavingDraft(null)}
        title={t.wfSaveAs}
        description={t.libSub}
        label={t.wfSaveAsPrompt}
        initial={savingDraft?.wf.name ?? ''}
        confirmLabel={t.projSave}
        cancelLabel={t.cancel}
        onSubmit={async (name) => {
          if (!savingDraft) return
          try {
            await createWorkflowEntry(token, {
              name,
              workflow: savingDraft.wf,
              layout: savingDraft.layout,
            })
            await reloadLibrary()
            toast(t.wfSavedToLibrary, 'success')
          } catch (e) {
            handleErr(e)
          }
        }}
      />

      <PromptDialog
        open={renaming !== null}
        onOpenChange={(o) => !o && setRenaming(null)}
        title={t.libRename}
        label={t.libRenamePrompt}
        initial={renaming?.name ?? ''}
        confirmLabel={t.projSave}
        cancelLabel={t.cancel}
        onSubmit={async (name) => {
          if (!renaming || name === renaming.name) return
          try {
            await patchWorkflowEntry(token, renaming.id, { name })
            await reloadLibrary()
          } catch (e) {
            handleErr(e)
          }
        }}
      />

      <ConfirmDialog
        open={deletingWorkflow !== null}
        onOpenChange={(o) => !o && setDeletingWorkflow(null)}
        title={t.confirmDeleteWfTitle}
        description={fill(t.confirmDeleteWfBody, { name: deletingWorkflow?.name ?? '' })}
        confirmLabel={t.confirmDeleteWfYes}
        cancelLabel={t.cancel}
        onConfirm={async () => {
          if (!deletingWorkflow) return
          try {
            await deleteWorkflowEntry(token, deletingWorkflow.id)
            await reloadLibrary()
          } catch (e) {
            handleErr(e)
          }
        }}
      />
    </AppShell>
  )
}

/**
 * A bordered aside that carries a consequence.
 *
 * `warn` is for the export: the file is a credential, and that sentence has to
 * survive being read at a glance by someone who came here to press one button.
 * `muted` is for a refusal, which is information rather than a hazard.
 */
function Callout({
  tone,
  title,
  children,
}: {
  tone: 'warn' | 'muted'
  title: string
  children: React.ReactNode
}) {
  return (
    <div
      className={
        tone === 'warn'
          ? 'border-l-2 border-[color:var(--accent2)] py-1 pl-3.5'
          : 'border-border-soft border-l-2 py-1 pl-3.5'
      }
    >
      <div className="text-foreground text-[13px] font-[680]">{title}</div>
      <div className="text-muted-foreground mt-0.5 max-w-prose text-[13px] leading-relaxed">
        {children}
      </div>
    </div>
  )
}
