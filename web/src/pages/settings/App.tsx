// /settings — the admin console.
//
// The page is new; almost nothing behind it is. Tokens, projects and the write
// budget have had endpoints since long before this page existed, and an operator
// reached them through the CLI or curl. `GET /v1/export/sqlite` is the one thing
// that had to be built for it, because "download the database" was the only
// admin capability with no HTTP surface at all.
//
// Everything here is admin-only, so the page states that plainly rather than
// rendering empty sections: a token without the scope gets one explanation and
// the command that mints a better one, not four failed requests.
import { useCallback, useEffect, useMemo, useState } from 'react'
import { useNavigate } from 'react-router'

import { AppHeader } from '@/components/AppHeader'
import { TokenGate } from '@/components/TokenGate'
import { Field } from '@/components/Field'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Input } from '@/components/ui/input'

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

/** `{name}`/`{size}`/`{id}` substitution — the string tables carry placeholders. */
function fill(template: string, values: Record<string, string>): string {
  return template.replace(/\{(\w+)\}/g, (m, k: string) => values[k] ?? m)
}

export function App() {
  const navigate = useNavigate()
  const { toast } = useToast()

  const [token, setToken] = useState(() => loadToken())
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem(LS_LANG)))
  const [gateError, setGateError] = useState('')

  const [who, setWho] = useState<Whoami | null>(null)
  const [projects, setProjects] = useState<Project[]>([])
  const [tokens, setTokens] = useState<TokenRow[]>([])

  const [exporting, setExporting] = useState(false)
  const [creatingToken, setCreatingToken] = useState(false)
  const [minted, setMinted] = useState<CreatedToken | null>(null)
  const [copied, setCopied] = useState(false)
  const [creatingProject, setCreatingProject] = useState(false)

  const t = useMemo(() => pick(STR, lang), [lang])

  const isAdmin = (who?.scopes ?? []).includes('admin')
  // An allowlist is what makes the whole-database export unavailable, so the page
  // can say why BEFORE the button is pressed rather than surfacing a 403 after.
  const allowlist = projectAllowlist(who)
  const scopedToProjects = allowlist !== null

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

  // A 403 from the export means "this token may not take a whole-database dump"
  // — a refusal of ONE operation by a token otherwise entitled to this page,
  // which `handleErr` now shows as a toast rather than a sign-out. This used to
  // need a hand-rolled bypass of `handleErr`; the rule lives in `isAuthError`.
  const onExport = async () => {
    setExporting(true)
    try {
      const { filename, bytes } = await downloadDatabase(token)
      toast(fill(t.exportDone, { name: filename, size: formatBytes(bytes) }), 'success')
    } catch (e) {
      handleErr(e)
    } finally {
      setExporting(false)
    }
  }

  const onRevoke = async (row: TokenRow) => {
    if (!window.confirm(t.confirmRevoke)) return
    try {
      await revokeToken(token, row.id)
      await refresh()
    } catch (e) {
      handleErr(e)
    }
  }

  const onDeleteProject = async (p: Project) => {
    if (!window.confirm(fill(t.confirmDeleteProject, { id: p.id }))) return
    try {
      await deleteProject(token, p.id)
      await refresh()
    } catch (e) {
      handleErr(e)
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

      <main className="mx-auto flex w-full max-w-3xl flex-col gap-5 overflow-y-auto px-5 py-6">
        <Card>
          <CardHeader>
            <CardTitle className="text-base">{t.sessionTitle}</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col gap-2 text-[13px]">
            <Row label={t.sessionActor}>
              <span className="font-mono">{who?.actor ?? '…'}</span>
            </Row>
            <Row label={t.sessionScopes}>
              <span className="flex flex-wrap gap-1">
                {(who?.scopes ?? []).map((s) => (
                  <Badge key={s} variant="secondary">
                    {s}
                  </Badge>
                ))}
              </span>
            </Row>
            <Row label={t.sessionProjects}>
              {scopedToProjects ? (
                <span className="font-mono">{(allowlist ?? []).join(', ')}</span>
              ) : (
                <span className="text-muted-foreground">{t.allProjects}</span>
              )}
            </Row>
          </CardContent>
        </Card>

        {!isAdmin && (
          <Card>
            <CardContent className="text-muted-foreground pt-5 text-[13px] leading-relaxed">
              {t.notAdmin}
            </CardContent>
          </Card>
        )}

        {isAdmin && (
          <>
            <Card>
              <CardHeader>
                <CardTitle className="text-base">{t.exportTitle}</CardTitle>
              </CardHeader>
              <CardContent className="flex flex-col gap-3 text-[13px] leading-relaxed">
                <p className="text-muted-foreground">{t.exportBody}</p>
                <p className="border-l-2 border-[color:var(--accent2)] pl-3 font-medium">
                  {t.exportWarn}
                </p>
                {scopedToProjects ? (
                  <p className="text-muted-foreground">{t.exportScoped}</p>
                ) : (
                  <div>
                    <Button onClick={() => void onExport()} disabled={exporting}>
                      {exporting ? t.exportBusy : t.exportBtn}
                    </Button>
                  </div>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between gap-3">
                <CardTitle className="text-base">{t.tokensTitle}</CardTitle>
                <Button variant="secondary" onClick={() => setCreatingToken((v) => !v)}>
                  + {t.tokensNew}
                </Button>
              </CardHeader>
              <CardContent className="flex flex-col gap-3 text-[13px]">
                <p className="text-muted-foreground">{t.tokensSub}</p>

                {minted && (
                  <div className="bg-muted flex flex-col gap-2 rounded-lg p-3">
                    <div className="font-medium">{t.tokenCreatedOnce}</div>
                    <code className="break-all font-mono text-[12px]">{minted.token}</code>
                    <div className="flex gap-2">
                      <Button
                        variant="secondary"
                        onClick={() => {
                          void navigator.clipboard?.writeText(minted.token)
                          setCopied(true)
                        }}
                      >
                        {copied ? t.tokenCopied : t.tokenCopy}
                      </Button>
                      <Button
                        variant="ghost"
                        onClick={() => {
                          setMinted(null)
                          setCopied(false)
                        }}
                      >
                        {t.tokenDone}
                      </Button>
                    </div>
                  </div>
                )}

                {creatingToken && (
                  <NewTokenForm
                    t={t}
                    onCancel={() => setCreatingToken(false)}
                    onCreate={async (fields) => {
                      try {
                        const created = await createToken(token, fields)
                        setMinted(created)
                        setCopied(false)
                        setCreatingToken(false)
                        await refresh()
                      } catch (e) {
                        handleErr(e)
                      }
                    }}
                  />
                )}

                {tokens.length === 0 ? (
                  <p className="text-muted-foreground">{t.tokensEmpty}</p>
                ) : (
                  <ul className="flex flex-col gap-2">
                    {tokens.map((row) => (
                      <li
                        key={row.id}
                        className="border-b-border-soft flex flex-wrap items-center gap-2 border-b pb-2 last:border-b-0"
                      >
                        <span className="min-w-0 flex-1 font-mono break-all">{row.actor}</span>
                        <span className="flex flex-wrap gap-1">
                          {row.scopes.map((s) => (
                            <Badge key={s} variant="secondary">
                              {s}
                            </Badge>
                          ))}
                          {row.projects !== '*' && (
                            <Badge variant="secondary">{row.projects.join(', ')}</Badge>
                          )}
                          {row.revoked_at && <Badge>{t.tokenRevoked}</Badge>}
                        </span>
                        <span className="text-muted-foreground text-[11.5px]">
                          {row.last_used_at
                            ? `${t.tokenLastUsed} ${row.last_used_at.slice(0, 10)}`
                            : t.tokenNeverUsed}
                        </span>
                        {!row.revoked_at && (
                          <Button variant="ghost" onClick={() => void onRevoke(row)}>
                            {t.tokenRevoke}
                          </Button>
                        )}
                      </li>
                    ))}
                  </ul>
                )}
              </CardContent>
            </Card>

            <Card>
              <CardHeader className="flex flex-row items-center justify-between gap-3">
                <CardTitle className="text-base">{t.projectsTitle}</CardTitle>
                <Button variant="secondary" onClick={() => setCreatingProject((v) => !v)}>
                  + {t.projectsNew}
                </Button>
              </CardHeader>
              <CardContent className="flex flex-col gap-3 text-[13px]">
                <p className="text-muted-foreground">{t.projectsSub}</p>

                {creatingProject && (
                  <NewProjectForm
                    t={t}
                    onCancel={() => setCreatingProject(false)}
                    onCreate={async (fields) => {
                      try {
                        await createProject(token, fields)
                        setCreatingProject(false)
                        await refresh()
                      } catch (e) {
                        handleErr(e)
                      }
                    }}
                  />
                )}

                {projects.length === 0 ? (
                  <p className="text-muted-foreground">{t.projectsEmpty}</p>
                ) : (
                  <ul className="flex flex-col gap-2">
                    {projects.map((p) => (
                      <li
                        key={p.id}
                        className="border-b-border-soft flex flex-wrap items-center gap-2 border-b pb-2 last:border-b-0"
                      >
                        <span className="min-w-0 flex-1 font-mono break-all">{p.id}</span>
                        <span className="text-muted-foreground">{p.name ?? ''}</span>
                        {p.workflow && <Badge variant="secondary">{p.workflow}</Badge>}
                        <Button variant="ghost" onClick={() => void onDeleteProject(p)}>
                          {t.projectDelete}
                        </Button>
                      </li>
                    ))}
                  </ul>
                )}
              </CardContent>
            </Card>
          </>
        )}
      </main>
    </div>
  )
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <span className="text-muted-foreground w-20 shrink-0 text-[10.5px] font-bold tracking-[0.05em] uppercase">
        {label}
      </span>
      {children}
    </div>
  )
}

type Strings = ReturnType<typeof pick<(typeof STR)['en']>>

function NewTokenForm({
  t,
  onCreate,
  onCancel,
}: {
  t: Strings
  onCreate: (f: { actor: string; scopes: string[]; projects?: string[] | null }) => Promise<void>
  onCancel: () => void
}) {
  const [actor, setActor] = useState('')
  const [scopes, setScopes] = useState('read,write')
  const [projects, setProjects] = useState('')

  const submit = () => {
    const list = scopes
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean)
    const proj = projects
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean)
    if (!actor.trim() || list.length === 0) return
    // Blank means "every project", which is `null` on the wire — an empty array
    // would be an allowlist permitting nothing.
    void onCreate({ actor: actor.trim(), scopes: list, projects: proj.length ? proj : null })
  }

  return (
    <div className="bg-muted flex flex-col gap-3 rounded-lg p-3">
      <div className="flex flex-wrap gap-3 [&>*]:flex-[1_1_170px]">
        <Field label={t.tokenActor}>
          {(id) => (
            <Input id={id} value={actor} onChange={(e) => setActor(e.target.value)} autoFocus />
          )}
        </Field>
        <Field label={t.tokenScopes}>
          {(id) => <Input id={id} value={scopes} onChange={(e) => setScopes(e.target.value)} />}
        </Field>
        <Field label={t.tokenProjects}>
          {(id) => (
            <Input id={id} value={projects} onChange={(e) => setProjects(e.target.value)} />
          )}
        </Field>
      </div>
      <div className="flex gap-2">
        <Button onClick={submit}>{t.tokenCreate}</Button>
        <Button variant="ghost" onClick={onCancel}>
          {t.tokenCancel}
        </Button>
      </div>
    </div>
  )
}

function NewProjectForm({
  t,
  onCreate,
  onCancel,
}: {
  t: Strings
  onCreate: (f: { id: string; name: string; workflow?: string }) => Promise<void>
  onCancel: () => void
}) {
  const [id, setId] = useState('')
  const [name, setName] = useState('')
  const [workflow, setWorkflow] = useState('')

  const submit = () => {
    if (!id.trim()) return
    const fields: { id: string; name: string; workflow?: string } = {
      id: id.trim(),
      name: name.trim() || id.trim(),
    }
    if (workflow.trim()) fields.workflow = workflow.trim()
    void onCreate(fields)
  }

  return (
    <div className="bg-muted flex flex-col gap-3 rounded-lg p-3">
      <div className="flex flex-wrap gap-3 [&>*]:flex-[1_1_170px]">
        <Field label={t.projectId}>
          {(fid) => (
            <Input id={fid} value={id} onChange={(e) => setId(e.target.value)} autoFocus />
          )}
        </Field>
        <Field label={t.projectName}>
          {(fid) => <Input id={fid} value={name} onChange={(e) => setName(e.target.value)} />}
        </Field>
        <Field label={t.projectWorkflow}>
          {(fid) => (
            <Input id={fid} value={workflow} onChange={(e) => setWorkflow(e.target.value)} />
          )}
        </Field>
      </div>
      <div className="flex gap-2">
        <Button onClick={submit}>{t.projectCreate}</Button>
        <Button variant="ghost" onClick={onCancel}>
          {t.tokenCancel}
        </Button>
      </div>
    </div>
  )
}
