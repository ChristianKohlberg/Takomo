// The frame every surface sits in: the nav rail on the left, the page to the
// right of it.
//
// Each page used to open with the same `flex h-dvh flex-col overflow-hidden`
// root and then render its own header. That root is now here, so adding the
// rail did not mean editing five layouts — and the next thing that belongs to
// the whole app rather than to one surface has one place to go.
//
// `min-w-0` on the content column is load-bearing rather than tidy: a flex item
// defaults to `min-width: auto`, so a wide table or a long unbroken ticket title
// inside it would refuse to shrink and push the page sideways instead of
// scrolling within itself.
import { useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import { DiagramContext } from '@/lib/diagram'
import { listQuestions } from '@/lib/questions'
import { loadToken } from '@/lib/session'
import { ProjectUpdatesContext } from '@/hooks/useProjectUpdates'
import type { Locale } from '@/lib/i18n'
import { NavRail, type NavRailProps } from './NavRail'

export interface AppShellProps {
  hideRail?: boolean
  lang?: Locale
  onLang?: (lang: Locale) => void
  /** Everything the rail needs; see NavRail. */
  rail: NavRailProps
  /** The surface: its header and its body. */
  children: ReactNode
}

export function AppShell({ rail, children, lang, onLang, hideRail = false }: AppShellProps) {
  const token = loadToken()
  const project = rail.project ?? ''
  const explicitCount = rail.badges?.inbox
  const shared = useContext(ProjectUpdatesContext)
  const [inbox, setInbox] = useState<{ scope: string; count?: number } | null>(null)
  const request = useRef(0)
  const scope = `${token}:${project}`
  const refreshInbox = useCallback(async () => {
    if (!token || explicitCount != null) return
    const generation = ++request.current
    try {
      const questions = await listQuestions(token, { project, status: 'open' })
      if (generation === request.current) setInbox({ scope, count: questions.length })
    } catch {
      if (generation === request.current) setInbox({ scope })
    }
  }, [token, project, scope, explicitCount])
  useEffect(() => {
    if (!token || explicitCount != null) return
    const requests = request
    void refreshInbox()
    const refresh = () => { void refreshInbox() }
    window.addEventListener('focus', refresh)
    const timer = setInterval(refresh, 30_000)
    return () => { ++requests.current; clearInterval(timer); window.removeEventListener('focus', refresh) }
  }, [refreshInbox, token, explicitCount])
  useEffect(() => {
    if (!token || explicitCount != null || !shared || shared.project !== project) return
    return shared.subscribe(refreshInbox)
  }, [shared, project, token, explicitCount, refreshInbox])
  const navigation = { ...rail, badges: { ...rail.badges, inbox: explicitCount ?? (inbox?.scope === scope ? inbox.count : undefined) } }
  return (
    <DiagramContext value={{ token, project }}>
    <div className="flex h-dvh overflow-hidden">
      <div style={{ display: hideRail ? 'none' : 'contents' }}>
        <NavRail {...navigation} lang={lang} onLang={onLang} navigationInHeader />
      </div>
      <div className="flex min-w-0 grow flex-col overflow-hidden">{children}</div>
    </div>
    </DiagramContext>
  )
}
