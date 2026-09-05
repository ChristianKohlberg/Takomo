import type { Locale } from '@/lib/i18n'

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown> : undefined
}

function text(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined
}

/** References describe intended coverage; only test runs establish results. */
export function CodeReferences({ metadata, lang }: { metadata: unknown; lang: Locale }) {
  const specification = record(record(metadata)?.specification)
  const raw = specification?.bindings
  if (raw === undefined || raw === null || (Array.isArray(raw) && raw.length === 0)) return null
  const references = Array.isArray(raw) ? raw.flatMap(value => {
    const binding = record(value)
    const file = text(binding?.file)
    const selector = text(binding?.selector)
    return file && selector ? [{ file, selector, proves: text(binding?.proves), limits: text(binding?.limits) }] : []
  }) : []
  const malformed = !Array.isArray(raw) || references.length !== raw.length
  const source = text(specification?.bindings_source_commit)
  const de = lang === 'de'
  return <details className="mt-3 min-w-0 text-sm">
    <summary className="cursor-pointer rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2">{de ? 'Code-Referenzen' : 'Code references'} ({references.length})</summary>
    <div className="mt-2 grid min-w-0 gap-3">
      <p className="text-muted-foreground">{de ? 'Diese Zuordnungen beschreiben, was die Tests prüfen sollen. Sie sind keine Testergebnisse. Ergebnisse findest du in den Testläufen.' : 'These mappings describe what the tests are intended to check. They are not test results. See test runs for execution evidence.'}</p>
      {source && <p className="min-w-0 text-xs text-muted-foreground">{de ? 'Referenzstand' : 'References recorded at'}: <code className="break-all">{source}</code></p>}
      {malformed && <p role="status" className="text-muted-foreground">{de ? 'Einige Referenzen konnten nicht angezeigt werden: Dateipfad oder Testname fehlt.' : 'Some references could not be displayed: a file path or test selector is missing.'}</p>}
      <ul className="grid min-w-0 gap-3">{references.map((reference, index) => <li key={index} className="min-w-0 rounded-md bg-muted p-3">
        <p><code className="break-all text-xs">{reference.file}</code></p>
        <p className="mt-1 break-words font-medium">{reference.selector}</p>
        <dl className="mt-2 grid min-w-0 gap-1">
          <dt className="text-xs font-medium">{de ? 'Was wird geprüft?' : 'What it checks'}</dt>
          <dd className="break-words text-muted-foreground">{reference.proves ?? (de ? 'Nicht beschrieben.' : 'Not described.')}</dd>
          <dt className="mt-1 text-xs font-medium">{de ? 'Was bleibt offen?' : 'What it leaves open'}</dt>
          <dd className="break-words text-muted-foreground">{reference.limits ?? (de ? 'Keine Grenzen dokumentiert.' : 'No limitations documented.')}</dd>
        </dl>
      </li>)}</ul>
    </div>
  </details>
}
