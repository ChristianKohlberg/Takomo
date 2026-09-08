import { useState } from 'react'
import { Button } from '@/components/ui/button'
import type { Locale } from '@/lib/i18n'

/** Human instructions stay readable; runner-specific data remains available. */
export function CaseDetails({ assignment, lang }: { assignment: unknown; lang: Locale }) {
  const fields = assignment !== null && typeof assignment === 'object' && !Array.isArray(assignment)
    ? assignment as Record<string, unknown> : undefined
  const steps = Array.isArray(fields?.steps) && fields.steps.length > 0 && fields.steps.every(step => typeof step === 'string' && step.trim())
    ? fields.steps as string[] : undefined
  const expected = typeof fields?.expected === 'string' && fields.expected.trim() ? fields.expected : undefined
  const parameters = fields
    ? Object.fromEntries(Object.entries(fields).filter(([key]) => !(key === 'steps' && steps) && !(key === 'expected' && expected)))
    : assignment
  const hasParameters = fields ? Object.keys(parameters as Record<string, unknown>).length > 0 : assignment !== null && assignment !== undefined
  const de = lang === 'de'
  const [copyStatus, setCopyStatus] = useState('')
  const labelFor = (key: string) => key === 'environment' ? (de ? 'Umgebung' : 'Environment') : key.replace(/_/g, ' ').replace(/^./, value => value.toUpperCase())
  const readable = (value: unknown): string => {
    if (Array.isArray(value)) return value.map(readable).join(', ')
    if (value !== null && typeof value === 'object') return Object.entries(value).map(([key, item]) => `${labelFor(key)}: ${readable(item)}`).join(' · ')
    return String(value ?? '—')
  }
  return <div className="mt-2 grid min-w-0 gap-3 text-sm">
    {steps && <div className="min-w-0">
      <p className="font-medium">{de ? 'Schritte' : 'Steps'}</p>
      <ol className="mt-1 list-decimal space-y-1 pl-5">{steps.map((step, index) => <li key={index} className="whitespace-pre-wrap break-words">{step}</li>)}</ol>
    </div>}
    {expected && <div className="min-w-0">
      <p className="font-medium">{de ? 'Erwartetes Ergebnis' : 'Expected result'}</p>
      <p className="mt-1 whitespace-pre-wrap break-words">{expected}</p>
    </div>}
    {hasParameters && <details className="min-w-0">
      <summary className="cursor-pointer rounded-sm text-muted-foreground focus-visible:outline-2 focus-visible:outline-offset-2">{de ? 'Parameter' : 'Parameters'}</summary>
      {fields && <dl className="mt-2 grid min-w-0 gap-2">{Object.entries(parameters as Record<string, unknown>).filter(([key]) => key !== 'steps' && key !== 'expected').map(([key, value]) => <div key={key} className="min-w-0 rounded-md border p-2">
        <dt className="text-xs font-medium">{labelFor(key)}</dt>
        <dd className="mt-1 flex min-w-0 flex-wrap items-start gap-2 text-xs"><span className="min-w-0 flex-1 whitespace-pre-wrap break-all">{readable(value)}</span>
          {typeof value === 'string' && value && <Button variant="ghost" size="sm" aria-label={`${de ? 'Kopieren' : 'Copy'}: ${labelFor(key)}`} onClick={() => { if (!navigator.clipboard) { setCopyStatus(de ? 'Kopieren fehlgeschlagen' : 'Copy failed'); return } void navigator.clipboard.writeText(value).then(() => setCopyStatus(de ? 'Kopiert' : 'Copied')).catch(() => setCopyStatus(de ? 'Kopieren fehlgeschlagen' : 'Copy failed')) }}>{de ? 'Kopieren' : 'Copy'}</Button>}
        </dd>
      </div>)}</dl>}
      {copyStatus && <p role="status" className="mt-2 text-xs">{copyStatus}</p>}
      <details className="mt-2"><summary className="cursor-pointer text-xs text-muted-foreground">{de ? 'Rohdaten (JSON)' : 'Raw data (JSON)'}</summary>
      <pre className="mt-2 max-w-full whitespace-pre-wrap break-all text-xs text-muted-foreground">{JSON.stringify(parameters, null, 2)}</pre></details>
    </details>}
  </div>
}
