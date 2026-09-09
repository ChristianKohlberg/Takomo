import { useState } from 'react'
import { resolveDocumentNumbering, type DocumentAppearance } from '@/lib/document-appearance'

export type NumberingVisibility = { h1: boolean; h2: boolean }
type Override = Partial<NumberingVisibility>
const keyFor = (project: string) => `takomo:document-numbering:${project}`
function read(project: string): Override {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(keyFor(project)) ?? '{}')
    if (!value || typeof value !== 'object') return {}
    const record = value as Record<string, unknown>
    return { ...(typeof record.h1 === 'boolean' ? { h1: record.h1 } : {}), ...(typeof record.h2 === 'boolean' ? { h2: record.h2 } : {}) }
  } catch { return {} }
}

/** A reader's display preference never changes the shared document or project. */
export function useDocumentNumbering(project: string, appearance?: DocumentAppearance) {
  const [saved, setSaved] = useState(() => ({ project, overrides: read(project) }))
  let overrides = saved.overrides
  if (saved.project !== project) {
    overrides = read(project)
    setSaved({ project, overrides })
  }
  const defaults = resolveDocumentNumbering(appearance)
  const setOverride = (level: keyof NumberingVisibility, value: boolean) => {
    const next = { ...overrides, [level]: value }
    setSaved({ project, overrides: next })
    try { localStorage.setItem(keyFor(project), JSON.stringify(next)) } catch { /* Display remains usable without storage. */ }
  }
  const reset = () => {
    setSaved({ project, overrides: {} })
    try { localStorage.removeItem(keyFor(project)) } catch { /* Display remains usable without storage. */ }
  }
  return { value: { h1: overrides.h1 ?? defaults.h1, h2: overrides.h2 ?? defaults.h2 }, overridden: Object.keys(overrides).length > 0, setOverride, reset }
}

export function DocumentNumberingControls({ value, overridden, setOverride, reset, locale }: ReturnType<typeof useDocumentNumbering> & { locale: 'en' | 'de' }) {
  const de = locale === 'de'
  return <fieldset className="flex min-w-0 flex-wrap items-center gap-2" aria-label={de ? 'Abschnittsnummern' : 'Section numbers'}>
    <legend className="sr-only">{de ? 'Abschnittsnummern' : 'Section numbers'}</legend>
    <span className="text-muted-foreground text-xs">{de ? 'Nummern' : 'Numbers'}</span>
    {(['h1', 'h2'] as const).map(level => <button key={level} type="button" aria-pressed={value[level]}
      aria-label={`${de ? 'Nummern für' : 'Numbers for'} ${level.toUpperCase()}`}
      className="border-input hover:bg-accent rounded border px-2 py-1 text-xs aria-pressed:bg-accent aria-pressed:font-semibold"
      onClick={() => setOverride(level, !value[level])}>{level.toUpperCase()}</button>)}
    {overridden && <button type="button" onClick={reset} className="text-muted-foreground hover:text-foreground text-xs underline underline-offset-2">
      {de ? 'Projektvorgaben verwenden' : 'Use project defaults'}
    </button>}
  </fieldset>
}
