import { CheckCircle2, Clock3, LoaderCircle, TriangleAlert, Unplug, RefreshCw, XIcon } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Dialog, DialogTrigger, DialogContent, DialogTitle, DialogDescription, DialogClose } from '@/components/ui/dialog'
import { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider } from '@/components/ui/tooltip'
import { useEmbeddingStatus } from '@/hooks/useEmbeddingStatus'
import { embeddingState, EMBEDDING_LABELS } from '@/lib/embedding-status'
import type { Locale } from '@/lib/i18n'

export function DocumentEmbeddingStatus({ locale, canSync }: { locale: Locale; canSync: boolean }) {
  const store = useEmbeddingStatus()
  const [open, setOpen] = useState(false)
  const watch = store?.watch
  useEffect(() => open && watch ? watch() : undefined, [open, watch])
  if (!store) return null
  const { status, error, syncing, deferred, localPending, awaitingFreshStatus, embed } = store
  const de = locale === 'de'
  const state = embeddingState(status, error, localPending, awaitingFreshStatus)
  const label = EMBEDDING_LABELS[locale][state]
  const Icon = state === 'current' ? CheckCircle2 : state === 'running' || state === 'loading' ? LoaderCircle : state === 'pending' ? Clock3 : state === 'unconfigured' ? Unplug : state === 'error' ? TriangleAlert : RefreshCw
  const time = status?.last_synced_at != null && Number.isFinite(status.last_synced_at) ? new Date(status.last_synced_at) : null
  const title = de ? 'Dokument-Embeddings' : 'Document embeddings'
  return <Dialog open={open} onOpenChange={value => { if (value) store.clearNotice(); setOpen(value) }}>
    <TooltipProvider><Tooltip><TooltipTrigger asChild><DialogTrigger asChild>
      <button type="button" aria-label={label} aria-haspopup="dialog" className={`flex size-10 shrink-0 items-center justify-center rounded-md hover:bg-muted ${state === 'current' ? 'text-emerald-700 dark:text-emerald-400' : state === 'error' ? 'text-destructive' : 'text-muted-foreground'}`}>
        <Icon className={`size-4 ${state === 'running' || state === 'loading' ? 'animate-spin motion-reduce:animate-none' : ''}`} aria-hidden="true" />
      </button>
    </DialogTrigger></TooltipTrigger><TooltipContent>{label}</TooltipContent></Tooltip></TooltipProvider>
    <DialogContent showCloseButton={false} className="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-md">
      <div className="flex items-center justify-between gap-3"><DialogTitle>{title}</DialogTitle><DialogClose className="flex size-10 shrink-0 items-center justify-center rounded hover:bg-muted" aria-label={de ? 'Schließen' : 'Close'}><XIcon className="size-4" aria-hidden="true" /></DialogClose></div>
      <DialogDescription>{de ? 'Stand der Bedeutungssuche für dieses Dokument.' : 'Meaning-search coverage for this document.'}</DialogDescription>
      <p role="status" className="text-sm font-medium">{label}</p>
      {status && <dl className="divide-y rounded-lg border px-3 text-sm">
        <div className="flex items-center justify-between gap-4 py-3"><dt>{de ? 'Passagen mit Embeddings' : 'Passage embeddings'}</dt><dd className="font-medium tabular-nums">{status.passages_indexed} / {status.passages_total}</dd></div>
        <div className="flex items-center justify-between gap-4 py-3"><dt>{de ? 'Abschnitte ausstehend' : 'Sections pending'}</dt><dd className="tabular-nums">{status.pending}</dd></div>
        <div className="flex items-center justify-between gap-4 py-3"><dt>{de ? 'Abschnitte in Arbeit' : 'Sections running'}</dt><dd className="tabular-nums">{status.running}</dd></div>
        <div className="flex items-center justify-between gap-4 py-3"><dt>{de ? 'Abschnitte fehlgeschlagen' : 'Sections failed'}</dt><dd className="tabular-nums">{status.failed}</dd></div>
        <div className="space-y-1 py-3"><dt>{de ? 'Letzte erfolgreiche Synchronisierung' : 'Last successful sync'}</dt><dd className="text-muted-foreground">{time && !Number.isNaN(time.getTime()) ? <time dateTime={time.toISOString()}>{new Intl.DateTimeFormat(de ? 'de-DE' : 'en-GB', { dateStyle: 'medium', timeStyle: 'short' }).format(time)}</time> : (de ? 'Noch nicht erfasst' : 'Not yet recorded')}</dd></div>
      </dl>}
      {localPending && <p role="status" className="text-sm text-muted-foreground">{de ? 'Änderungen sind noch nicht auf dem Server gespeichert. Embeddings werden danach aktualisiert.' : 'Changes have not reached the server yet. Embeddings will update after saving.'}</p>}
      {(error || status?.last_error) && <p role="alert" className="text-sm break-words text-destructive">{error || status?.last_error}</p>}
      {deferred && <p role="status" className="text-sm text-muted-foreground">{de ? 'Das Dokument ändert sich noch. Nichts wurde vorgemerkt; bitte nach einer Schreibpause erneut versuchen.' : 'The document is still changing. Nothing was scheduled; try again after typing pauses.'}</p>}
      {state === 'unconfigured' && <a className="text-sm underline" href="/settings?section=search">{de ? 'Bedeutungssuche konfigurieren' : 'Configure meaning search'}</a>}
      {canSync && <div className="space-y-2"><button type="button" disabled={!status?.configured || syncing || localPending} className="min-h-10 rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground disabled:opacity-50" onClick={() => { void embed() }}>{syncing ? (de ? 'Wird vorgemerkt…' : 'Scheduling…') : (de ? 'Jetzt einbetten' : 'Embed now')}</button><p className="text-xs text-muted-foreground">{de ? 'Unveränderte Passagen werden wiederverwendet.' : 'Unchanged passages are reused.'}</p></div>}
    </DialogContent>
  </Dialog>
}
