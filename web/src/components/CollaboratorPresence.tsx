import { UsersIcon } from 'lucide-react'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import type { Locale } from '@/lib/i18n'

/** Presence names identify sessions, not verified people or agent roles. */
export function CollaboratorPresence({ peers, lang }: { peers: string[]; lang: Locale }) {
  if (!peers.length) return null
  const names = new Map<string, number>()
  peers.forEach(name => names.set(name, (names.get(name) ?? 0) + 1))
  const label = lang === 'de' ? 'Anwesende' : 'Collaborators'
  return <Popover><PopoverTrigger asChild><button type="button" aria-label={`${label}: ${names.size}`} className="text-muted-foreground hover:bg-muted flex shrink-0 items-center gap-1.5 rounded-lg px-2 py-1 text-xs">
    <UsersIcon size={15} /><span>{names.size}</span>
  </button></PopoverTrigger><PopoverContent align="end" className="max-w-[calc(100vw-2rem)]">
    <h2 className="mb-2 font-semibold">{label}</h2>
    <ul className="max-h-64 space-y-2 overflow-auto">{[...names].map(([name, count]) => <li key={name} className="flex min-w-0 items-center gap-2 text-sm"><span aria-hidden className="bg-secondary text-secondary-foreground flex size-7 shrink-0 items-center justify-center rounded-full">{name.slice(0, 1).toUpperCase()}</span><span className="min-w-0 flex-1 break-words">{name}</span><span className="text-muted-foreground text-xs">{count} {lang === 'de' ? (count === 1 ? 'Sitzung' : 'Sitzungen') : (count === 1 ? 'session' : 'sessions')}</span></li>)}</ul>
  </PopoverContent></Popover>
}
