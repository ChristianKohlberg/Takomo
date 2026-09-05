import { CheckIcon, CloudOffIcon, LoaderCircleIcon, TriangleAlertIcon, LockKeyholeIcon } from 'lucide-react'
import type { SaveState } from '@/lib/save-status'
import type { Locale } from '@/lib/i18n'

const text = {
  en: { connecting: 'Connecting…', syncing: 'Syncing…', saved: 'Saved', offline: 'Offline · saved on this device', 'local-error': 'Local backup unavailable', 'server-error': 'Not saved to server · retrying', 'read-only': 'Read only' },
  de: { connecting: 'Verbinden…', syncing: 'Synchronisieren…', saved: 'Gespeichert', offline: 'Offline · auf diesem Gerät gespeichert', 'local-error': 'Lokale Sicherung nicht verfügbar', 'server-error': 'Nicht auf dem Server gespeichert · neuer Versuch', 'read-only': 'Nur lesen' },
}
export function SaveStatus({ state, lang }: { state: SaveState; lang: Locale }) {
  const waiting = state === 'syncing' || state === 'connecting'
  const error = state === 'local-error' || state === 'server-error'
  const Icon = waiting ? LoaderCircleIcon : error ? TriangleAlertIcon : state === 'offline' ? CloudOffIcon : state === 'read-only' ? LockKeyholeIcon : CheckIcon
  return <span role="status" className={`inline-flex min-w-0 items-center gap-1.5 text-xs ${error ? 'text-destructive' : state === 'saved' ? 'text-emerald-700 dark:text-emerald-400' : 'text-muted-foreground'}`}>
    <Icon aria-hidden className={`size-3.5 flex-none ${waiting ? 'animate-spin' : ''}`} />
    <span>{text[lang][state]}</span>
  </span>
}
