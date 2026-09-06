import { useId, useRef, useState } from 'react'
import { api, type ApiErrorShape } from '@/lib/api'
import { isAuthError } from '@/lib/session'
import type { Ticket } from '@/lib/board'
import { pick, type Locale } from '@/lib/i18n'
import { Button } from '@/components/ui/button'
import { STR } from './strings'
export const fieldClass = 'w-full min-w-0 rounded-lg border bg-background px-3 py-2 text-sm'
export function Report({ token, project, lang, onCreated, onCancel, onAuthError }: {token: string; project: string; lang: Locale; onCreated: (ticket: Ticket) => void; onCancel: () => void; onAuthError?: () => void}) {
  const t = pick(STR, lang)
  const descriptionHint = useId()
  const [draft, setDraft] = useState({ title: '', body: '', steps: '', expected: '', actual: '' })
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const request = useRef<{ key: string; payload: string } | null>(null)
  return <form className="space-y-3 rounded-xl border bg-card p-4" onSubmit={event => {
    event.preventDefault()
    if (busy || !draft.title.trim() || !(draft.body.trim() || draft.actual.trim())) return
    // Once sent, retain the exact request through ambiguous errors. Retrying
    // cannot duplicate a ticket or accidentally reuse a key with edited data.
    request.current ??= { key: crypto.randomUUID(), payload: JSON.stringify({ project, type: 'bug', title: draft.title.trim(), priority: 'normal', body: [draft.body, `## ${t.steps}\n${draft.steps}`, `## ${t.expected}\n${draft.expected}`, `## ${t.actual}\n${draft.actual}`].join('\n\n') }) }
    setBusy(true); setError('')
    void api<Ticket>(token, '/tickets', { method: 'POST', headers: { 'Content-Type': 'application/json', 'Idempotency-Key': request.current.key }, body: request.current.payload }).then(onCreated).catch(e => { if (isAuthError(e)) onAuthError?.(); const status = (e as ApiErrorShape)?.status; if (status && status >= 400 && status < 500 && status !== 408) request.current = null; setError(e instanceof Error ? e.message : String(e)) }).finally(() => setBusy(false))
  }}>
    <h2 className="font-semibold">{t.report}</h2><p id={descriptionHint} className="text-sm text-muted-foreground">{t.reportRequired}</p>
    {(['title', 'body', 'steps', 'expected', 'actual'] as const).map(key => <label key={key} className="block space-y-1 text-sm"><span>{key === 'title' ? t.bugTitle : t[key]}</span>{key === 'title' ? <input required className={fieldClass} value={draft[key]} disabled={busy || !!request.current} onChange={e => setDraft({ ...draft, [key]: e.target.value })} /> : <textarea aria-describedby={descriptionHint} required={key === 'body' ? !draft.actual.trim() : key === 'actual' ? !draft.body.trim() : false} className={fieldClass} value={draft[key]} disabled={busy || !!request.current} onChange={e => setDraft({ ...draft, [key]: e.target.value })} />}</label>)}
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    <div className="flex flex-wrap gap-2"><Button disabled={busy || !draft.title.trim() || !(draft.body.trim() || draft.actual.trim())} type="submit">{t.create}</Button><Button variant="outline" type="button" disabled={busy} onClick={onCancel}>{t.cancel}</Button></div>
  </form>
}
