import { useEffect, useId, useRef, useState } from 'react'
import { SearchIcon, XIcon } from 'lucide-react'
import { Dialog, DialogClose, DialogContent, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { excerptRanges, searchDocument, searchStatus, syncSearch, type SearchResult, type SearchResponse, type SearchStatus } from '@/lib/hybrid-search'
import type { Locale } from '@/lib/i18n'

export function SearchExcerpt({ result }: { result: SearchResult }) {
  const ranges = excerptRanges(result)
  let end = 0
  return <>{ranges.map(range => {
    const prefix = result.excerpt.slice(end, range.from)
    end = range.to
    return <span key={range.from}>{prefix}<mark className="rounded bg-yellow-200 text-neutral-950 dark:bg-yellow-700 dark:text-white">{result.excerpt.slice(range.from, range.to)}</mark></span>
  })}{result.excerpt.slice(end)}</>
}

export function DocumentHybridSearch({ token, map, locale, canSync, onNavigate }: {
  token: string; map: string; locale: Locale; canSync: boolean; onNavigate: (result: SearchResult) => void
}) {
  const de = locale === 'de'
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [response, setResponse] = useState<SearchResponse | null>(null)
  const [status, setStatus] = useState<SearchStatus | null>(null)
  const [error, setError] = useState('')
  const [statusError, setStatusError] = useState('')
  const [busy, setBusy] = useState(false)
  const [syncing, setSyncing] = useState(false)
  const [active, setActive] = useState(0)
  const trigger = useRef<HTMLButtonElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const previousFocus = useRef<HTMLElement | null>(null)
  const navigating = useRef<SearchResult | null>(null)
  const listId = useId()
  const results = response?.results ?? []
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.key.toLowerCase() !== 's') return
      if (document.querySelector('[role="dialog"]') && !open) return
      event.preventDefault()
      if (!open) previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null
      setOpen(true)
    }
    window.addEventListener('keydown', key)
    return () => window.removeEventListener('keydown', key)
  }, [open])
  useEffect(() => {
    if (!open) return
    const controller = new AbortController()
    let timer: ReturnType<typeof setTimeout>
    const refresh = () => {
      searchStatus(token, map, controller.signal).then(value => {
        if (controller.signal.aborted) return
        setStatus(value); setStatusError('')
        timer = setTimeout(refresh, 3000)
      }).catch((e: Error) => { if (!controller.signal.aborted) setStatusError(e.message) })
    }
    refresh()
    return () => { controller.abort(); clearTimeout(timer) }
  }, [open, token, map])
  useEffect(() => {
    if (!open || !query.trim()) return
    const controller = new AbortController()
    const timer = setTimeout(() => {
      searchDocument(token, map, query.trim(), controller.signal).then(value => {
        if (!controller.signal.aborted) { setResponse(value); setBusy(false) }
      }).catch((e: Error) => { if (!controller.signal.aborted) { setError(e.message); setBusy(false) } })
    }, 200)
    return () => { controller.abort(); clearTimeout(timer) }
  }, [open, query, token, map])
  useEffect(() => {
    document.getElementById(`${listId}-${active}`)?.scrollIntoView?.({ block: 'nearest' })
  }, [active, listId])
  const choose = (result: SearchResult) => { navigating.current = result; setOpen(false) }
  const title = de ? 'Dokument durchsuchen' : 'Search document'
  const state = statusError ? (de ? 'Indexstatus nicht verfügbar' : 'Index status unavailable')
    : !status ? (de ? 'Indexstatus wird geladen…' : 'Loading index status…')
    : !status.configured ? (de ? 'Bedeutungssuche nicht konfiguriert · Stichwortsuche verfügbar' : 'Meaning search not configured · keyword search available')
    : status.last_error ? (de ? 'Indexfehler · Stichwortsuche verfügbar' : 'Index error · keyword search available')
    : status.running > 0 ? (de ? 'Index wird aktualisiert…' : 'Updating index…')
    : status.queued > 0 ? (de ? 'Aktualisierung vorgemerkt' : 'Update pending')
    : status.indexed < status.total ? (de ? 'Index unvollständig' : 'Index incomplete')
    : (de ? 'Index aktuell' : 'Index current')
  return <>
    <button ref={trigger} type="button" aria-haspopup="dialog" className="flex min-h-9 items-center gap-1.5 rounded px-2 py-1 text-sm hover:bg-muted" onClick={() => { previousFocus.current = trigger.current; setOpen(true) }}>
      <SearchIcon className="size-4" aria-hidden="true" />{title}<kbd className="hidden text-xs text-muted-foreground sm:inline">⌘ / Ctrl S</kbd>
    </button>
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent onOpenAutoFocus={event => { event.preventDefault(); input.current?.focus() }} showCloseButton={false} className="flex max-h-[calc(100dvh-2rem)] flex-col gap-3 sm:max-w-2xl" onCloseAutoFocus={event => {
        event.preventDefault()
        const result = navigating.current
        navigating.current = null
        if (result) onNavigate(result)
        else (previousFocus.current?.isConnected ? previousFocus.current : trigger.current)?.focus()
      }}>
        <div className="flex items-center justify-between gap-3"><DialogTitle>{title}</DialogTitle><DialogClose className="flex size-9 shrink-0 items-center justify-center rounded hover:bg-muted" aria-label={de ? 'Suche schließen' : 'Close search'}><XIcon className="size-4" aria-hidden="true" /></DialogClose></div>
        <DialogDescription>{de ? 'Mit Stichwörtern oder einer Beschreibung suchen. ↑ ↓ auswählen, Enter öffnen.' : 'Search by keywords or describe what you need. ↑ ↓ to select, Enter to open.'}</DialogDescription>
        <input ref={input} aria-label={title} role="combobox" aria-autocomplete="list" aria-expanded={results.length > 0} aria-controls={listId} aria-activedescendant={results[active] ? `${listId}-${active}` : undefined}
          maxLength={500} value={query} className="w-full rounded-lg border bg-background px-3 py-3 text-base outline-offset-2" placeholder={de ? 'Was suchst du?' : 'What are you looking for?'}
          onChange={event => { setQuery(event.target.value); setResponse(null); setError(''); setActive(0); setBusy(Boolean(event.target.value.trim())) }}
          onKeyDown={event => {
            if (event.nativeEvent.isComposing) return
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setActive(index => results.length ? (index + (event.key === 'ArrowDown' ? 1 : -1) + results.length) % results.length : 0) }
            if (event.key === 'Enter' && results[active]) { event.preventDefault(); choose(results[active]) }
          }} />
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <span role="status">{state}{status?.configured ? ` · ${status.indexed}/${status.total}` : ''}</span>
          {canSync && <button type="button" disabled={syncing || !status?.configured} className="min-h-9 rounded border px-2 text-foreground disabled:opacity-50" onClick={() => {
            setSyncing(true); setStatusError('')
            void syncSearch(token, map).then(setStatus).catch((e: Error) => setStatusError(e.message)).finally(() => setSyncing(false))
          }}>{syncing ? (de ? 'Wird vorgemerkt…' : 'Scheduling…') : (de ? 'Dokument synchronisieren' : 'Sync document')}</button>}
          {!status?.configured && <a className="underline" href="/settings?section=search">{de ? 'Suche konfigurieren' : 'Configure search'}</a>}
        </div>
        {(statusError || status?.last_error) && <p className="text-xs text-destructive" role="status">{statusError || status?.last_error}</p>}
        {response?.mode === 'keyword' && response.semantic_status === 'unavailable' && status?.configured && !status.last_error && <p className="text-xs text-muted-foreground">{de ? 'Stichwortergebnisse · Bedeutungssuche derzeit nicht verfügbar.' : 'Keyword results · meaning search is currently unavailable.'}</p>}
        <div aria-live="polite" className="text-sm text-muted-foreground">{error || (busy ? (de ? 'Suche läuft…' : 'Searching…') : response ? `${results.length} ${de ? (results.length === 1 ? 'Ergebnis' : 'Ergebnisse') : (results.length === 1 ? 'result' : 'results')}` : '')}</div>
        <div id={listId} role="listbox" aria-label={de ? 'Suchergebnisse' : 'Search results'} className="min-h-0 overflow-y-auto overscroll-contain rounded-lg border empty:hidden">
          {results.map((result, index) => <button key={result.node_id} id={`${listId}-${index}`} type="button" role="option" aria-selected={index === active} tabIndex={-1} onClick={() => choose(result)} onPointerMove={() => setActive(index)}
            className={`block w-full border-b p-3 text-left last:border-b-0 ${index === active ? 'bg-accent text-accent-foreground' : 'hover:bg-muted'}`}>
            <span className="block text-xs text-muted-foreground break-words">{(result.heading_path.at(-1) === result.title ? result.heading_path.slice(0, -1) : result.heading_path).join(' › ')}</span>
            <span className="block font-semibold break-words">{result.title || (de ? 'Ohne Titel' : 'Untitled section')}</span>
            {result.match_kind === 'semantic' && <span className="text-xs text-muted-foreground">{de ? 'Ähnliche Bedeutung' : 'Related meaning'}</span>}
            <span className="mt-1 block whitespace-pre-wrap text-sm break-words"><SearchExcerpt result={result} /></span>
          </button>)}
        </div>
      </DialogContent>
    </Dialog>
  </>
}
