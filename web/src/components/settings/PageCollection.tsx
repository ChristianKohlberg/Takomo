import { Link } from 'react-router'
import { ArrowUpRightIcon } from 'lucide-react'
import type { Locale } from '@/lib/i18n'

/** Secondary workspaces stay available to every signed-in reader; each route owns its permissions. */
export function PageCollection({ lang, project = '' }: { lang: Locale; project?: string }) {
  const de = lang === 'de'
  const pages = [
    ['/agent-queues', de ? 'Agenten-Queue' : 'Agent queue', de ? 'Agentenaufträge ansehen und verwalten' : 'View and manage agent work'],
    ['/bugs', de ? 'Fehler' : 'Bugs', de ? 'Fehlerberichte und Recherche' : 'Bug reports and research'],
    ['/epics', 'Epics', de ? 'Zusammengehörige Aufgaben verwalten' : 'Manage related tasks'],
    ['/initiatives', de ? 'Initiativen' : 'Initiatives', de ? 'Ideen und größere Vorhaben verwalten' : 'Manage ideas and larger plans'],
    ['/schedules', de ? 'Zeitpläne' : 'Schedules', de ? 'Wiederkehrende Aufgaben verwalten' : 'Manage recurring tasks'],
    ['/environments', de ? 'Umgebungen' : 'Environments', de ? 'Testumgebungen ansehen und verwalten' : 'View and manage test environments'],
  ]
  return <section aria-labelledby="page-collection-title" className="space-y-3">
    <h2 id="page-collection-title" className="text-lg font-semibold">{de ? 'Weitere Seiten' : 'More pages'}</h2>
    <div className="grid gap-2 sm:grid-cols-2">
      {pages.map(([path, label, description]) => <Link key={path} to={project ? `${path}?project=${encodeURIComponent(project)}` : path!} className="flex items-center gap-3 rounded-lg border border-border-soft p-3 text-foreground hover:bg-muted focus-visible:outline-2 focus-visible:outline-primary">
        <div className="min-w-0 flex-1"><span className="font-semibold">{label}</span><p className="mt-1 text-xs text-muted-foreground">{description}</p></div><ArrowUpRightIcon size={16} className="shrink-0" />
      </Link>)}
    </div>
  </section>
}
