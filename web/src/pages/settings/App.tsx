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
// names" — and the page needed scrolling before it had any content. A rail makes
// the shape legible and gives each section the whole panel.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'

import { AppHeader } from '@/components/AppHeader'
import { TokenGate } from '@/components/TokenGate'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { ConfirmDialog } from '@/components/settings/ConfirmDialog'
import { NewProjectDialog } from '@/components/settings/NewProjectDialog'
import { NewTokenDialog } from '@/components/settings/NewTokenDialog'
import { TokenList } from '@/components/settings/TokenList'
import { TokenRevealDialog } from '@/components/settings/TokenRevealDialog'
import {
  EmptyState,
  FactRow,
  Section,
  SectionNav,
  type SectionDef,
} from '@/components/settings/SettingsShell'

import { isAuthError, loadToken, saveToken } from '@/lib/session'
import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { whoami, listProjects, type Project, type Whoami } from '@/lib/initiatives'
import {
  createProject,
  createToken,
  deleteProject,
  downloadDatabase,
  formatBytes,
  listTokens,
  projectAllowlist,
  revokeToken,
  type CreatedToken,
  type TokenRow,
} from '@/lib/admin'
import { STR } from './strings'

const LS_LANG = 'takomo.lang'
const LS_SECTION = 'takomo.settings.section'

type SectionKey = 'overview' | 'data' | 'access' | 'projects'

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
  // usually the same reason.
  const [section, setSection] = useState<SectionKey>(() => {
    const stored = localStorage.getItem(LS_SECTION)
    return stored === 'data' || stored === 'access' || stored === 'projects' ? stored : 'overview'
  })

  const [who, setWho] = useState<Whoami | null>(null)
  const [projects, setProjects] = useState<Project[]>([])
  const [tokens, setTokens] = useState<TokenRow[]>([])

  const [exporting, setExporting] = useState(false)
  const [newToken, setNewToken] = useState(false)
  const [minted, setMinted] = useState<CreatedToken | null>(null)
  const [newProject, setNewProject] = useState(false)
  const [revoking, setRevoking] = useState<TokenRow | null>(null)
  const [deleting, setDeleting] = useState<Project | null>(null)

  const t = useMemo(() => pick(STR, lang), [lang])

  const isAdmin = (who?.scopes ?? []).includes('admin')
  // An allowlist is what makes the whole-database export unavailable, so the page
  // says why BEFORE the button is pressed rather than surfacing a 403 after.
  const allowlist = projectAllowlist(who)
  const scopedToProjects = allowlist !== null

  const SECTIONS: readonly SectionDef<SectionKey>[] = [
    { key: 'overview', label: t.navOverview, hint: t.navOverviewHint },
    { key: 'data', label: t.navData, hint: t.navDataHint },
    { key: 'access', label: t.navAccess, hint: t.navAccessHint },
    { key: 'projects', label: t.navProjects, hint: t.navProjectsHint },
  ]

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

  const refresh = useCallback(async () => {
    const [ps, ts] = await Promise.all([
      listProjects(token).catch(() => [] as Project[]),
      listTokens(token).catch(() => [] as TokenRow[]),
    ])
    setProjects(ps)
    setTokens(ts)
  }, [token])

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

  // Deliberately NOT routed through `handleErr`.
  //
  // `isAuthError` counts 403 as an auth failure and signs the viewer out, which
  // is right for the polls it was written for: a revoked token must not read as
  // a flaky network. It is wrong here. A 403 from the export means "this token
  // may not take a whole-database dump" — a refusal of ONE operation by a token
  // that is otherwise entitled to this page. Throwing that person back to the
  // gate would log them out of a console they can legitimately use, and the
  // reason they came would never be shown.
  const onExport = async () => {
    setExporting(true)
    try {
      const { filename, bytes } = await downloadDatabase(token)
      toast(fill(t.dataDone, { name: filename, size: formatBytes(bytes) }), 'success')
    } catch (e) {
      if ((e as { status?: number })?.status === 401) {
        handleErr(e)
        return
      }
      toast((e as { message?: string })?.message || t.requestFailed, 'err')
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
    <div className="flex h-dvh flex-col overflow-hidden">
      <AppHeader
        onNavigate={navigate}
        current="settings"
        nav={{
          board: t.board,
          inbox: t.inbox,
          initiatives: t.initiatives,
          schedules: t.schedules,
          settings: t.settings,
        }}
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
      >
        <Button variant="ghost" onClick={signOut}>
          {t.signOut}
        </Button>
      </AppHeader>

      <div className="mx-auto flex w-full max-w-5xl flex-1 flex-col gap-6 overflow-y-auto px-5 py-6 md:flex-row md:gap-10">
        {isAdmin && (
          <SectionNav
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
          ) : (
            <Section
              title={t.projTitle}
              description={t.projSub}
              action={<Button onClick={() => setNewProject(true)}>+&nbsp;{t.projNew}</Button>}
            >
              {projects.length === 0 ? (
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
                      <Button variant="destructive" size="sm" onClick={() => setDeleting(p)}>
                        {t.projDelete}
                      </Button>
                    </li>
                  ))}
                </ul>
              )}
            </Section>
          )}
        </div>
      </div>

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
            await refresh()
          } catch (e) {
            handleErr(e)
          }
        }}
      />
    </div>
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
