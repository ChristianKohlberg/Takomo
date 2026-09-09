import { useEffect, useMemo, useRef, useState } from 'react'
import { ArrowDownIcon, ArrowUpIcon, SearchIcon, XIcon } from 'lucide-react'
import { searchDocument, type SearchResponse } from '@/lib/hybrid-search'
import { ancestorsOf } from '@/lib/mindmap-doc'
import type { MapNode } from '@/lib/mindmap-doc'
import type { Locale } from '@/lib/i18n'

/** Search visibility is local and temporary; never write it into saved folds. */
export function searchFolds(nodes: MapNode[], collapsed: ReadonlySet<string>, matches: ReadonlySet<string>) {
  const folds = new Set(collapsed)
  for (const id of matches) for (const parent of ancestorsOf(nodes, id)) folds.delete(parent)
  return folds
}

export function useMindmapSearch(token: string, map: string, nodes: MapNode[]) {
  const [query, setQuery] = useState('')
  const [result, setResult] = useState<{ query: string; token: string; map: string; response?: SearchResponse; error?: string } | null>(null)
  useEffect(() => {
    if (!query.trim()) return
    const controller = new AbortController()
    const timer = setTimeout(() => {
      searchDocument(token, map, query.trim(), controller.signal).then(response => {
        if (!controller.signal.aborted) setResult({ query, token, map, response })
      }).catch((error: Error) => {
        if (!controller.signal.aborted) setResult({ query, token, map, error: error.message })
      })
    }, 200)
    return () => { controller.abort(); clearTimeout(timer) }
  }, [query, token, map])
  const current = query.trim() && result?.query === query && result.token === token && result.map === map ? result : null
  const response = current?.response
  const matches = useMemo(() => {
    const existing = new Set(nodes.map(node => node.id))
    return new Set(response?.results.map(result => result.node_id).filter(id => existing.has(id)))
  }, [response, nodes])
  return { query, setQuery: (value: string) => { setResult(null); setQuery(value) }, response, matches, error: current?.error, busy: Boolean(query.trim() && !current) }
}

type Props = ReturnType<typeof useMindmapSearch> & { locale: Locale; onNavigate: (id: string) => void }
export function MindmapSearch({ query, setQuery, response, matches, error, busy, locale, onNavigate }: Props) {
  const de = locale === 'de'
  const input = useRef<HTMLInputElement>(null)
  const [active, setActive] = useState<string | null>(null)
  const ids = [...matches]
  const index = active ? ids.indexOf(active) : -1
  const move = (direction: number) => {
    const next = ids[(index < 0 ? (direction > 0 ? 0 : ids.length - 1) : index + direction + ids.length) % ids.length]
    if (next) { setActive(next); onNavigate(next) }
  }
  useEffect(() => {
    const key = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey || event.key.toLowerCase() !== 's' || document.querySelector('[role="dialog"]')) return
      event.preventDefault(); input.current?.focus(); input.current?.select()
    }
    window.addEventListener('keydown', key)
    return () => window.removeEventListener('keydown', key)
  }, [])
  const label = de ? 'Dokument durchsuchen' : 'Search document'
  return <div className="border-b-border-soft flex flex-wrap items-center gap-2 border-b px-4 py-2" role="search" onKeyDown={event => event.stopPropagation()}>
    <div className="relative min-w-0 flex-1 sm:max-w-sm">
      <SearchIcon aria-hidden="true" className="text-muted-foreground absolute top-2.5 left-2 size-4" />
      <input ref={input} type="search" aria-label={label} placeholder={label} maxLength={500} value={query} className="border-border bg-card min-h-9 w-full rounded-md border pr-2 pl-8 text-sm [&::-webkit-search-cancel-button]:appearance-none" onChange={event => { setActive(null); setQuery(event.target.value) }} onKeyDown={event => {
        if (event.nativeEvent.isComposing) return
        if (event.key === 'Escape') { setQuery(''); setActive(null) }
        if (event.key === 'Enter') { event.preventDefault(); move(event.shiftKey ? -1 : 1) }
      }} />
    </div>
    {query && <button type="button" aria-label={de ? 'Suche löschen' : 'Clear search'} className="hover:bg-muted flex size-9 items-center justify-center rounded" onClick={() => { setQuery(''); setActive(null); input.current?.focus() }}><XIcon aria-hidden="true" className="size-4" /></button>}
    {ids.length > 0 && <>
      <button type="button" aria-label={de ? 'Vorheriger Treffer' : 'Previous match'} className="hover:bg-muted flex size-9 items-center justify-center rounded" onClick={() => move(-1)}><ArrowUpIcon aria-hidden="true" className="size-4" /></button>
      <button type="button" aria-label={de ? 'Nächster Treffer' : 'Next match'} className="hover:bg-muted flex size-9 items-center justify-center rounded" onClick={() => move(1)}><ArrowDownIcon aria-hidden="true" className="size-4" /></button>
    </>}
    <span role="status" className="text-muted-foreground text-xs">{error || (busy ? (de ? 'Suche läuft…' : 'Searching…') : response ? `${index >= 0 ? `${index + 1} / ` : ''}${ids.length} ${de ? 'Treffer' : 'matches'}${response.truncated ? (de ? ' · Beste Ergebnisse' : ' · Top results') : ''}` : '')}
      {response?.projection === 'stale' && (de ? ' · Ergebnisse möglicherweise veraltet' : ' · Results may be out of date')}
      {response?.mode === 'keyword' && (de ? ' · Stichwortsuche' : ' · Keyword search')}
    </span>
  </div>
}
