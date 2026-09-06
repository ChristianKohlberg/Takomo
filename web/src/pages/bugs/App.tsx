import { useCallback, useEffect, useState } from 'react'
import { useNavigate, useSearchParams } from 'react-router'
import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { TokenGate } from '@/components/TokenGate'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { listProjects, whoami, type Project } from '@/lib/initiatives'
import { DEFAULT_BUGS_VIEW, writeBugsView } from '@/lib/bugs-url'
import { isAuthError, loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { STR as COMMON } from '../environments/strings'
import { STR } from './strings'
import { Queue } from './Queue'

export default function App() {
  const navigate = useNavigate()
  const [params, setParams] = useSearchParams()
  const [token, setToken] = useState(loadToken)
  const [stored, setStored] = useState(loadProject)
  const linked = params.get('project')
  const project = linked ?? stored
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem('takomo.lang')))
  const [navCollapsed, setNavCollapsed] = useNavCollapsed()
  const [identity, setIdentity] = useState<{ actor: string; scopes: string[]; projects: Project[] } | null>(null)
  const [error, setError] = useState('')
  const [gateError, setGateError] = useState('')
  const t = pick(STR, lang)
  const c = pick(COMMON, lang)
  const signOut = useCallback(() => { saveToken(''); setToken(''); setIdentity(null); setError('') }, [])
  useEffect(() => {
    if (linked === null && stored) setParams(current => { const next = new URLSearchParams(current); next.set('project', stored); return next }, { replace: true })
  }, [linked, stored, setParams])
  useEffect(() => {
    if (!token) return
    let cancelled = false
    void (async () => {
      try {
        const [who, projects] = await Promise.all([whoami(token), listProjects(token)])
        if (cancelled) return
        if (!(who.scopes ?? []).includes('read')) { signOut(); setGateError(c.gateNoRead); return }
        setIdentity({ actor: who.actor ?? '', scopes: who.scopes ?? [], projects })
        setError('')
      } catch (e) {
        if (cancelled) return
        if (isAuthError(e)) signOut()
        else setError(e instanceof Error ? e.message : c.requestFailed)
      }
    })()
    return () => { cancelled = true }
  }, [token, signOut, c.gateNoRead, c.requestFailed])
  if (!token) return <TokenGate title={`takomo · ${t.title}`} subtitle={c.tokenNeeded} tokenLabel={c.gateLabel} openLabel={c.gateOpen} emptyMessage={c.tokenNeeded} error={gateError} onSubmit={value => { saveToken(value); setToken(value); setGateError('') }} />
  return <AppShell lang={lang} onLang={value => { setLang(value); localStorage.setItem('takomo.lang', value) }} rail={{
    current: 'bugs', onNavigate: navigate,
    nav: { board: c.board, epics: c.epics, inbox: c.inbox, specification: c.specification, initiatives: c.initiatives, schedules: c.schedules, environments: c.environments, bugs: t.title },
    projects: identity?.projects ?? [], project, onProject: value => { saveProject(value); setStored(value); setParams(writeBugsView({ ...DEFAULT_BUGS_VIEW, project: value })) },
    projectLabels: { project: c.project, search: c.projectSearch, noMatch: c.projectNoMatch, all: c.allProjects },
    labels: { expand: c.navExpand, collapse: c.navCollapse, signOut: c.signOut, account: c.navAccount, settings: c.settings },
    collapsed: navCollapsed, onCollapsed: setNavCollapsed, actor: identity?.actor, scopes: identity?.scopes, onSignOut: signOut,
  }}>
    <AppHeader title={t.title} />
    <main className="min-h-0 flex-1 overflow-y-auto p-4 md:p-5">
      {error && <p role="alert" className="text-destructive mb-4 break-words text-sm">{error}</p>}
      {identity ? <Queue canWrite={identity.scopes.includes('write')} canReview={identity.scopes.includes('write') && identity.scopes.includes('human')} canConfigure={identity.scopes.includes('admin')} key={`${token}:${project}`} token={token} project={project} lang={lang} onAuthError={signOut} /> : !error && <p role="status" className="text-muted-foreground">{t.loading}</p>}
    </main>
  </AppShell>
}
