import { useEffect, useState } from 'react'
import type { Locale } from '@/lib/i18n'
import { getProjectDocumentLinks, type ReferencePage } from '@/lib/ticket-document-links'
import { DOCUMENT_LINKS } from '@/components/board/document-link-strings'
interface Props { token: string; project: string; section: string; lang: Locale }
export function SectionTicketLinks(props: Props) { return <Links key={`${props.token}:${props.project}:${props.section}`} {...props} /> }
function Links({ token, project, section, lang }: Props) {
  const t = DOCUMENT_LINKS[lang]; const [open, setOpen] = useState(false); const [data, setData] = useState<ReferencePage | null>(null); const [error, setError] = useState(''); const [offset, setOffset] = useState(0); const [loading, setLoading] = useState(false); const [refresh, setRefresh] = useState(0)
  useEffect(() => {
    if (!open) return
    const controller = new AbortController(); setLoading(true); setError('')
    getProjectDocumentLinks(token, project, controller.signal, section, offset).then(result => { if (!controller.signal.aborted) setData(previous => offset === 0 || !previous ? result : { ...result, items: [...previous.items, ...result.items.filter(item => !previous.items.some(old => old.id === item.id))] }) }).catch(cause => { if (!controller.signal.aborted) setError(String(cause)) }).finally(() => { if (!controller.signal.aborted) setLoading(false) })
    return () => controller.abort()
  }, [token, project, section, open, offset, refresh])
  return <div className="text-xs"><button type="button" className="text-muted-foreground hover:text-foreground" aria-expanded={open} onClick={() => { setError(''); if (!open) { setOffset(0); setData(null) } setOpen(value => !value) }}>{t.tickets}{data ? ` (${data.total})` : ''}</button>
    {open && <div className="mt-1 max-h-48 max-w-full overflow-y-auto rounded border border-border-soft bg-card p-2">{error && <p role="alert">{error}<button type="button" className="ml-2 underline" onClick={() => setRefresh(value => value + 1)}>{t.refresh}</button></p>}{!data ? !error && <p role="status">{t.loading}</p> : <><ul className="space-y-1">{data.items.map(link => <li key={link.id}><a className="text-primary underline" href={`/board?project=${encodeURIComponent(project)}#t=${encodeURIComponent(link.ticket)}`}>{link.ticket_title || link.ticket}</a> · {link.ticket_state}</li>)}</ul>{data.total > data.items.length && <div><p>{t.limited.replace('{n}', String(data.items.length)).replace('{total}', String(data.total))}</p><button type="button" className="mt-1 underline" disabled={loading} onClick={() => setOffset(data.items.length)}>{t.more}</button></div>}</>}</div>}
  </div>
}
