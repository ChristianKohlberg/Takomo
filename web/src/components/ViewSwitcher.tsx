import { useEffect } from 'react'
import { useLocation } from 'react-router'
// The three views of one specification.
//
// A map, a written plan and a set of tests are not three destinations — they are
// the same nodes drawn, composed as prose, and checked. So the rail carries ONE
// entry and the choice of view lives here, in the surface's own header, where a
// switch is cheap and reversible.
//
// Rendered as links rather than buttons on purpose: each view is a real URL that
// survives a reload, a bookmark and a share, which is what `#n=` handover between
// the map and the plan already relies on. A button that navigated by script would
// break all three.
import { NetworkIcon, FileTextIcon, ShieldCheckIcon } from 'lucide-react'

export type SpecView = 'map' | 'document' | 'tests'

export interface ViewSwitcherLabels {
  map: string
  document: string
  tests: string
}

export interface ViewSwitcherProps {
  current: SpecView
  labels: ViewSwitcherLabels
  /** Same-document navigation, so switching view does not reload the app. */
  onNavigate: (href: string) => void
}

const VIEWS: { id: SpecView; href: string; Icon: typeof NetworkIcon }[] = [
  { id: 'map', href: '/mindmaps', Icon: NetworkIcon },
  { id: 'document', href: '/documents', Icon: FileTextIcon },
  { id: 'tests', href: '/verification', Icon: ShieldCheckIcon },
]

export function ViewSwitcher({ current, labels, onNavigate }: ViewSwitcherProps) {
  const location = useLocation()
  const project = localStorage.getItem('takomo.project') ?? ''
  const key = (path: string) => `takomo.workspace:${project}:${path}`
  useEffect(() => { sessionStorage.setItem(`takomo.workspace:${project}:${location.pathname}`, location.hash) }, [project, location.pathname, location.hash])
  return (
    <nav
      aria-label={labels.map}
      className="border-border-soft bg-muted/40 flex flex-none items-center gap-0.5 rounded-lg border p-0.5"
    >
      {VIEWS.map(({ id, href, Icon }) => {
        const active = id === current
        const target = href + (active ? location.hash : sessionStorage.getItem(key(href)) ?? '')
        return (
          <a
            key={id}
            href={target}
            aria-current={active ? 'page' : undefined}
            aria-label={labels[id]}
            onClick={(e) => {
              // Let a modified click do what the browser would.
              if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return
              e.preventDefault()
              onNavigate(target)
            }}
            className={[
              'flex items-center gap-1.5 rounded-md px-2.5 py-1.5 text-[13px] font-[650] no-underline',
              active
                ? 'bg-card text-foreground shadow-sm'
                : 'text-muted-foreground hover:text-foreground',
            ].join(' ')}
          >
            <Icon className="size-4 flex-none" aria-hidden="true" />
            <span className="hidden md:inline">{labels[id]}</span>
          </a>
        )
      })}
    </nav>
  )
}
