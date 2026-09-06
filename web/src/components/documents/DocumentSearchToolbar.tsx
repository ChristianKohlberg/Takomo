import { useEffect, useRef } from 'react'
import { ChevronDown, ChevronUp, X } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { Locale } from '@/lib/i18n'
import '@/styles/document-search.css'

export interface DocumentSearchToolbarProps {
  query: string
  onQuery: (query: string) => void
  count: number
  activeIndex: number
  onNext: () => void
  onPrevious: () => void
  onClose: () => void
  locale: Locale
}

export function DocumentSearchToolbar({ query, onQuery, count, activeIndex, onNext, onPrevious, onClose, locale }: DocumentSearchToolbarProps) {
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => { input.current?.focus() }, [])
  const de = locale === 'de'
  const label = de ? 'Im Dokument suchen' : 'Find in document'
  return <div role="search" aria-label={label} className="flex min-w-0 flex-wrap items-center gap-2 border-b border-border bg-background px-3 py-2">
    <input ref={input} type="search" aria-label={label} placeholder={label} value={query}
      className="min-w-0 flex-1 rounded-md border border-input bg-background px-2 py-1 text-sm"
      onChange={event => onQuery(event.target.value)}
      onKeyDown={event => {
        if (event.key === 'Escape') { event.preventDefault(); onClose() }
        if (event.key === 'Enter') { event.preventDefault(); if (event.shiftKey) onPrevious(); else onNext() }
      }} />
    <span role="status" className="text-xs text-muted-foreground">
      {!query ? (de ? 'Suchbegriff eingeben' : 'Enter a search term') : !count ? (de ? 'Keine Treffer' : 'No matches') : `${activeIndex + 1} / ${count}`}
    </span>
    <Button variant="ghost" size="icon" disabled={!count} aria-label={de ? 'Vorheriger Treffer' : 'Previous match'} onClick={onPrevious}><ChevronUp /></Button>
    <Button variant="ghost" size="icon" disabled={!count} aria-label={de ? 'Nächster Treffer' : 'Next match'} onClick={onNext}><ChevronDown /></Button>
    <Button variant="ghost" size="icon" aria-label={de ? 'Suche schließen' : 'Close search'} onClick={onClose}><X /></Button>
  </div>
}
