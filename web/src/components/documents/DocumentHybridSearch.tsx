import { readSearchHistory, rememberSearch, searchHistoryKey, writeSearchHistory } from '@/lib/search-history'
import { EmbeddingStatusProvider, useEmbeddingStatus } from '@/hooks/useEmbeddingStatus'
import { useEffect, useId, useRef, useState } from 'react'
import { SearchIcon, XIcon } from 'lucide-react'
import { Dialog, DialogClose, DialogContent, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { excerptRanges, searchDocument, type SearchResult, type SearchResponse } from '@/lib/hybrid-search'
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

type Props = { userId?: string; project?: string; token: string; map: string; locale: Locale; canSync: boolean; onNavigate: (result: SearchResult) => void }
export function DocumentHybridSearch(props: Props) {
  const shared = useEmbeddingStatus()
  const scope = `${props.map}:${props.token}`
  return shared ? <SearchDialog key={scope} {...props} /> : <EmbeddingStatusProvider key={scope} token={props.token} map={props.map}><SearchDialog key={scope} {...props} /></EmbeddingStatusProvider>
}
type ScopedHistory = { key: string | null; history: string[] }
/** Identity resolves after mount, so the scope moves while the dialog is open; a pending
 *  (session-only) scope carries its entries into the resolved one, any other move reads fresh. */
function rescopeHistory(current: ScopedHistory, key: string | null): ScopedHistory {
  const carried = current.key === null && key !== null ? current.history : []
  return { key, history: carried.reduceRight((history, item) => rememberSearch(history, item), readSearchHistory(key)) }
}
function SearchDialog({ token, map, userId, project, locale, canSync, onNavigate }: Props) {
  const { status, error: statusError, syncing, deferred, localPending, awaitingFreshStatus, embed, clearNotice, watch } = useEmbeddingStatus()!
  const de = locale === 'de'
  const [open, setOpen] = useState(false)
  useEffect(() => { if (open) clearNotice() }, [open, clearNotice])
  useEffect(() => open ? watch() : undefined, [open, watch])
  const historyKey = searchHistoryKey(userId, project)
  const [scoped, setScoped] = useState<ScopedHistory>(() => rescopeHistory({ key: null, history: [] }, historyKey))
  if (scoped.key !== historyKey) setScoped(rescopeHistory(scoped, historyKey))
  const history = scoped.key === historyKey ? scoped.history : []
  const setHistory = (history: string[]) => setScoped({ key: historyKey, history })
  useEffect(() => writeSearchHistory(scoped.key, scoped.history), [scoped])
  const [query, setQuery] = useState('')
  const remember = (value: string) => setHistory(rememberSearch(history, value))
  const [response, setResponse] = useState<SearchResponse | null>(null)
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [active, setActive] = useState(0)
  const trigger = useRef<HTMLButtonElement>(null)
  const input = useRef<HTMLInputElement>(null)
  const previousFocus = useRef<HTMLElement | null>(null)
  const navigating = useRef<SearchResult | null>(null)
  const listId = useId()
  const results = response?.results ?? []
  const showingHistory = !query.trim()
  const optionCount = showingHistory ? history.length : results.length
  const changeQuery = (value: string) => { setQuery(value); setResponse(null); setError(''); setActive(0); setBusy(Boolean(value.trim())) }
  const chooseHistory = (value: string) => { remember(value); changeQuery(value); input.current?.focus() }
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
  const choose = (result: SearchResult) => { remember(query); navigating.current = result; setOpen(false) }
  const title = de ? 'Dokument durchsuchen' : 'Search document'
  const state = statusError ? (de ? 'Indexstatus nicht verfügbar' : 'Index status unavailable')
    : !status ? (de ? 'Indexstatus wird geladen…' : 'Loading index status…')
    : !status.configured ? (de ? 'Bedeutungssuche nicht konfiguriert · Stichwortsuche verfügbar' : 'Meaning search not configured · keyword search available')
    : status.failed > 0 ? (de ? `Indexierung für ${status.failed} ${status.failed === 1 ? 'Abschnitt' : 'Abschnitte'} aufgegeben · Stichwortsuche verfügbar` : `Indexing gave up on ${status.failed} ${status.failed === 1 ? 'section' : 'sections'} · keyword search available`)
    : status.last_error ? (de ? 'Indexfehler · Stichwortsuche verfügbar' : 'Index error · keyword search available')
    : status.projection === 'stale' ? (de ? 'Dokument ändert sich noch · Index holt auf' : 'Document still changing · index catching up')
    : localPending ? (de ? 'Änderungen werden gespeichert · Embeddings ausstehend' : 'Saving changes · embeddings pending')
    : status.running > 0 ? (de ? 'Index wird aktualisiert…' : 'Updating index…')
    : status.queued > 0 ? (de ? 'Aktualisierung vorgemerkt' : 'Update pending')
    : awaitingFreshStatus ? (de ? 'Indexstatus wird geprüft…' : 'Checking index status…')
    : status.passages_indexed !== status.passages_total || !Number.isFinite(status.passages_total) ? (de ? 'Index unvollständig' : 'Index incomplete')
    : (de ? 'Index aktuell' : 'Index current')
  return <>
    <button ref={trigger} type="button" aria-haspopup="dialog" className="flex min-h-9 items-center gap-1.5 rounded px-2 py-1 text-sm hover:bg-muted" onClick={() => { previousFocus.current = trigger.current; setOpen(true) }}>
      <SearchIcon className="size-4" aria-hidden="true" />{title}<kbd className="hidden text-xs text-muted-foreground sm:inline">⌘ / Ctrl S</kbd>
    </button>
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent onOpenAutoFocus={event => { event.preventDefault(); input.current?.focus() }} showCloseButton={false} className="flex h-[min(42rem,calc(100dvh-2rem))] max-h-[calc(100dvh-2rem)] flex-col gap-3 overflow-hidden sm:max-w-2xl" onCloseAutoFocus={event => {
        event.preventDefault()
        const result = navigating.current
        navigating.current = null
        if (result) onNavigate(result)
        else (previousFocus.current?.isConnected ? previousFocus.current : trigger.current)?.focus()
      }}>
        <div className="flex items-center justify-between gap-3"><DialogTitle>{title}</DialogTitle><DialogClose className="flex size-9 shrink-0 items-center justify-center rounded hover:bg-muted" aria-label={de ? 'Suche schließen' : 'Close search'}><XIcon className="size-4" aria-hidden="true" /></DialogClose></div>
        <DialogDescription>{de ? 'Mit Stichwörtern oder einer Beschreibung suchen. ↑ ↓ auswählen, Enter öffnen.' : 'Search by keywords or describe what you need. ↑ ↓ to select, Enter to open.'}</DialogDescription>
        <input ref={input} aria-label={title} role="combobox" aria-autocomplete="list" aria-expanded={optionCount > 0} aria-controls={listId} aria-activedescendant={active < optionCount ? `${listId}-${active}` : undefined}
          maxLength={500} value={query} className="w-full shrink-0 rounded-lg border bg-background px-3 py-3 text-base outline-offset-2" placeholder={de ? 'Was suchst du?' : 'What are you looking for?'}
          onChange={event => changeQuery(event.target.value)}
          onKeyDown={event => {
            if (event.nativeEvent.isComposing) return
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setActive(index => optionCount ? (index + (event.key === 'ArrowDown' ? 1 : -1) + optionCount) % optionCount : 0) }
            if (event.key === 'Enter') {
              event.preventDefault()
              if (showingHistory && history[active]) chooseHistory(history[active])
              else if (results[active]) choose(results[active])
              else if (query.trim()) remember(query)
            }
          }} />
        <div data-search-body className="min-h-0 flex-1 space-y-3 overflow-y-auto overscroll-contain">
        <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
          <span role="status">{state}{status?.configured ? ` · ${status.indexed}/${status.total}` : ''}</span>
          {canSync && <button type="button" disabled={syncing || !status?.configured || localPending} className="min-h-9 rounded border px-2 text-foreground disabled:opacity-50" onClick={() => { void embed() }}>{syncing ? (de ? 'Wird vorgemerkt…' : 'Scheduling…') : (de ? 'Dokument synchronisieren' : 'Sync document')}</button>}
          {!status?.configured && <a className="underline" href="/settings?section=search">{de ? 'Suche konfigurieren' : 'Configure search'}</a>}
        </div>
        {(statusError || status?.last_error) && <p className="text-xs text-destructive" role="status">{statusError || status?.last_error}</p>}
        {deferred && <p className="text-xs text-muted-foreground" role="status">{de ? 'Das Dokument ändert sich noch; die Synchronisierung wurde zurückgestellt. Nichts wurde vorgemerkt – bitte erneut versuchen, sobald das Tippen pausiert.' : 'Document is still changing; synchronization is deferred. Nothing was scheduled, so try again once typing pauses.'}</p>}
        {response?.mode === 'keyword' && response.semantic_status === 'unavailable' && status?.configured && !status.last_error && <p className="text-xs text-muted-foreground">{de ? 'Stichwortergebnisse · Bedeutungssuche derzeit nicht verfügbar.' : 'Keyword results · meaning search is currently unavailable.'}</p>}
        {response?.projection === 'stale' && <p className="text-xs text-destructive" role="status">{de ? 'Ergebnisse können veraltet sein · Quelle konnte nicht indexiert werden' : 'Results may be out of date · source could not be indexed'}{response.projection_error ? `: ${response.projection_error}` : ''}</p>}
        {response?.mode === 'keyword' && response.semantic_status === 'throttled' && <p className="text-xs text-muted-foreground">{de ? 'Stichwortergebnisse · Bedeutungssuche kurz pausiert (Abfragelimit erreicht).' : 'Keyword results · meaning search paused briefly (query limit reached).'}</p>}
        <div aria-live="polite" className="text-sm text-muted-foreground">{error || (busy ? (de ? 'Suche läuft…' : 'Searching…') : !response ? ''
          : response.truncated ? (de ? `Die ${results.length} besten von ${response.candidates} passenden Abschnitten` : `Top ${results.length} of ${response.candidates} matching sections`)
          : `${results.length} ${de ? (results.length === 1 ? 'Ergebnis' : 'Ergebnisse') : (results.length === 1 ? 'result' : 'results')}`)}</div>
        {showingHistory && <div className="flex items-center justify-between gap-2 text-sm">
          <span>{de ? 'Letzte Suchen' : 'Recent searches'}</span>
          {history.length > 0 && <button type="button" className="min-h-9 rounded px-2 underline hover:bg-muted" onClick={() => { setHistory([]); setActive(0); input.current?.focus() }}>{de ? 'Verlauf löschen' : 'Clear history'}</button>}
        </div>}
        {showingHistory && history.length === 0 && <p className="text-sm text-muted-foreground">{de ? 'Noch keine Suchen. Eine Suche eingeben, um zu beginnen.' : 'No recent searches. Type a query to begin.'}</p>}
        <div id={listId} role="listbox" aria-label={showingHistory ? (de ? 'Letzte Suchen' : 'Recent searches') : (de ? 'Suchergebnisse' : 'Search results')} className="min-h-0 overflow-y-auto overscroll-contain rounded-lg border empty:hidden">
          {showingHistory && history.map((value, index) => <button key={value} id={`${listId}-${index}`} type="button" role="option" aria-selected={index === active} tabIndex={-1} onClick={() => chooseHistory(value)} onPointerMove={() => setActive(index)} className={`block min-h-11 w-full break-words border-b p-3 text-left last:border-b-0 ${index === active ? 'bg-accent text-accent-foreground' : 'hover:bg-muted'}`}>{value}</button>)}
          {!showingHistory && results.map((result, index) => <button key={result.node_id} id={`${listId}-${index}`} type="button" role="option" aria-selected={index === active} tabIndex={-1} onClick={() => choose(result)} onPointerMove={() => setActive(index)}
            className={`block w-full border-b p-3 text-left last:border-b-0 ${index === active ? 'bg-accent text-accent-foreground' : 'hover:bg-muted'}`}>
            <span className="block text-xs text-muted-foreground break-words">{(result.heading_path.at(-1) === result.title ? result.heading_path.slice(0, -1) : result.heading_path).join(' › ')}</span>
            <span className="block font-semibold break-words">{result.title || (de ? 'Ohne Titel' : 'Untitled section')}</span>
            {result.match_kind === 'semantic' && <span className="text-xs text-muted-foreground">{de ? 'Ähnliche Bedeutung' : 'Related meaning'}</span>}
            <span className="mt-1 block whitespace-pre-wrap text-sm break-words"><SearchExcerpt result={result} /></span>
          </button>)}
        </div>
        </div>
      </DialogContent>
    </Dialog>
  </>
}
