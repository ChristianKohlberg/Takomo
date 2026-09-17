import { listReviews } from '@/lib/document-reviews'
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
import { useState, type ReactNode } from 'react'
import { DiagramContext } from '@/lib/diagram'
import { listQuestions } from '@/lib/questions'
import { loadToken } from '@/lib/session'
import { useLiveRefresh } from '@/hooks/useLiveRefresh'
import type { Locale } from '@/lib/i18n'
import { cn } from '@/lib/utils'
import { NavRail, type NavRailProps } from './NavRail'

export interface AppShellProps {
  contentClassName?: string
  hideRail?: boolean
  lang?: Locale
  onLang?: (lang: Locale) => void
  /** Everything the rail needs; see NavRail. */
  rail: NavRailProps
  /** The surface: its header and its body. */
  children: ReactNode
}

export function AppShell({ rail, children, lang, onLang, hideRail = false, contentClassName }: AppShellProps) {
  const token = loadToken()
  const project = rail.project ?? ''
  const explicitCount = rail.badges?.inbox
  const [inbox, setInbox] = useState<{ scope: string; count?: number } | null>(null)
  const scope = `${token}:${project}`
  useLiveRefresh({
    token, project, scope: `${scope}:${explicitCount ?? 'auto'}`, topics: ['inbox'],
    onError: () => setInbox({ scope }),
    load: async signal => {
      const [questions, reviews] = await Promise.all([explicitCount == null ? listQuestions(token, { project, status: 'open' }) : Promise.resolve(null), listReviews(token, project, 'needs_me', 0, signal)])
      if (!signal.aborted) setInbox({ scope, count: (explicitCount ?? questions?.length ?? 0) + reviews.total })
    },
  })
  const navigation = { ...rail, badges: { ...rail.badges, inbox: token ? (inbox?.scope === scope ? inbox.count : undefined) : explicitCount } }
  return (
    <DiagramContext value={{ token, project }}>
    <div className="flex h-dvh flex-col overflow-hidden md:flex-row">
      <div style={{ display: hideRail ? 'none' : 'contents' }}>
        <NavRail {...navigation} lang={lang} onLang={onLang} navigationInHeader />
      </div>
      <div className={cn("flex min-w-0 grow flex-col overflow-hidden", contentClassName)}>{children}</div>
    </div>
    </DiagramContext>
  )
}
