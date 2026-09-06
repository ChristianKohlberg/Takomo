import { useCallback, useEffect, useState } from 'react'
import { useNavigate } from 'react-router'
import { AppHeader } from '@/components/AppHeader'
import { AppShell } from '@/components/AppShell'
import { TokenGate } from '@/components/TokenGate'
import { useNavCollapsed } from '@/hooks/useNavCollapsed'
import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { listProjects, whoami, type Project } from '@/lib/initiatives'
import { isAuthError, loadProject, loadToken, saveProject, saveToken } from '@/lib/session'
import { STR as COMMON } from '../environments/strings'
import { STR } from './strings'
import { Workspace } from './Workspace'

export default function App() {
  const navigate = useNavigate()
  const [token, setToken] = useState(loadToken)
  const [project, setProject] = useState(loadProject)
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem('takomo.lang')))
  const [navCollapsed, setNavCollapsed] = useNavCollapsed()
  const [identity, setIdentity] = useState<{ actor: string; scopes: string[]; projects: Project[] } | null>(null)
  const [error, setError] = useState('')
  const [gateError, setGateError] = useState('')
  const t = pick(STR, lang)
  const c = pick(COMMON, lang)
  const signOut = useCallback(() => { saveToken(''); setToken(''); setIdentity(null); setError('') }, [])
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
    current: 'lanes', onNavigate: navigate,
    nav: { board: c.board, epics: c.epics, inbox: c.inbox, specification: c.specification, initiatives: c.initiatives, schedules: c.schedules, environments: c.environments, lanes: t.title },
    projects: identity?.projects ?? [], project, onProject: value => { setProject(value); saveProject(value) },
    projectLabels: { project: c.project, search: c.projectSearch, noMatch: c.projectNoMatch, all: c.allProjects },
    labels: { expand: c.navExpand, collapse: c.navCollapse, signOut: c.signOut, account: c.navAccount, settings: c.settings },
    collapsed: navCollapsed, onCollapsed: setNavCollapsed, actor: identity?.actor, scopes: identity?.scopes, onSignOut: signOut,
  }}>
    <AppHeader title={t.title} />
    <main className="min-h-0 flex-1 overflow-y-auto p-4 md:p-5">
      {error && <p role="alert" className="text-destructive mb-4 break-words text-sm">{error}</p>}
      {identity ? <Workspace key={`${token}:${project}`} token={token} project={project} lang={lang} canWrite={(identity.scopes.includes('write') || identity.scopes.includes('admin')) && !identity.projects.find(p => p.id === project)?.archived} canSend={identity.scopes.includes('admin') || (identity.scopes.includes('write') && identity.scopes.includes('human'))} onAuthError={signOut} /> : !error && <p role="status" className="text-muted-foreground">{t.loading}</p>}
    </main>
  </AppShell>
}
