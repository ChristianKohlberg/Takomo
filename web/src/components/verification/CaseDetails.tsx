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
      <pre className="mt-2 max-w-full whitespace-pre-wrap break-all text-xs text-muted-foreground">{JSON.stringify(parameters, null, 2)}</pre>
    </details>}
  </div>
}
