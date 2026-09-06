import { useCallback, useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { Markdown } from '@/components/Markdown'
import { listTickets, type Ticket } from '@/lib/board'
import { cancelHandoff, createHandoff, createLane, dispatchHandoff, getLane, listHandoffs, listLanes, setLaneTicket, updateLane, type Handoff, type HandoffDraft, type Lane, type Provider } from '@/lib/lanes'
import { isAuthError } from '@/lib/session'
import { pick, type Locale } from '@/lib/i18n'
import { STR } from './strings'
type Labels = typeof STR.en
const selectClass = 'border-input bg-background h-9 w-full min-w-0 rounded-lg border px-2 text-sm'
type Props = { token: string; project: string; lang: Locale; canWrite: boolean; canSend: boolean; onAuthError: () => void }

export function Workspace(props: Props) {
  const { token, project, lang, canWrite, onAuthError } = props
  const t = pick(STR, lang)
  const [lanes, setLanes] = useState<Lane[] | null>(null)
  const [total, setTotal] = useState(0)
  const [selected, setSelected] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [error, setError] = useState('')
  const [refresh, setRefresh] = useState(0)
  const [busy, setBusy] = useState(false)
  const active = useRef(true)
  useEffect(() => { active.current = true; return () => { active.current = false } }, [])
  const fail = useCallback((e: unknown) => { if (!active.current) return; if (isAuthError(e)) onAuthError(); else setError(e instanceof Error ? e.message : String(e)) }, [onAuthError])
  useEffect(() => {
    if (!project) return
    const controller = new AbortController()
    void listLanes(token, project, controller.signal).then(data => { if (!controller.signal.aborted) { setLanes(data.items); setTotal(data.total); setError('') } }).catch(e => { if (!controller.signal.aborted) fail(e) })
    return () => controller.abort()
  }, [token, project, refresh, fail])
  if (!project) return <p className="text-muted-foreground text-sm">{t.selectProject}</p>
  if (selected) return <LaneDetail key={selected} {...props} id={selected} onBack={() => { setSelected(null); setRefresh(n => n + 1) }} />
  return <div className="mx-auto max-w-5xl space-y-4">
    <div className="flex flex-wrap items-center justify-between gap-2"><span className="text-muted-foreground text-sm">{lanes?.length ?? '—'} {t.title}</span>{canWrite && <Button onClick={() => setCreating(true)}>{t.newLane}</Button>}</div>
    {error && <div role="alert" className="text-destructive break-words text-sm">{error} <Button variant="outline" onClick={() => setRefresh(n => n + 1)}>{t.retry}</Button></div>}
    {creating && <form className="bg-card space-y-3 rounded-xl border p-4" onSubmit={e => { e.preventDefault(); const form = new FormData(e.currentTarget); setBusy(true); void createLane(token, project, { title: String(form.get('title')).trim(), purpose: String(form.get('purpose')).trim() }).then(lane => { if (active.current) setSelected(lane.id) }).catch(fail).finally(() => { if (active.current) setBusy(false) }) }}>
      <label className="block text-sm">{t.laneTitle}<Input name="title" required maxLength={200} /></label><label className="block text-sm">{t.purpose}<Textarea name="purpose" /></label><div className="flex gap-2"><Button disabled={busy}>{t.create}</Button><Button type="button" variant="ghost" onClick={() => setCreating(false)}>{t.cancel}</Button></div>
    </form>}
    {!lanes && !error && <p role="status">{t.loading}</p>}{lanes?.length === 0 && <p className="text-muted-foreground py-8 text-sm">{t.empty}</p>}
    <ul className="divide-y rounded-xl border">{lanes?.map(lane => <li key={lane.id}><button onClick={() => setSelected(lane.id)} className="hover:bg-muted focus-visible:ring-ring flex w-full min-w-0 flex-col gap-1 p-4 text-left focus-visible:ring-2"><span className="break-words font-semibold">{lane.title} {lane.archived && <span className="text-muted-foreground text-xs">{t.archived}</span>}</span><span className="text-muted-foreground line-clamp-2 break-words text-sm">{lane.purpose}</span><span className="text-muted-foreground text-xs">{lane.tickets?.length ?? 0} {t.tickets} · {lane.handoff_count} {t.handoffs}</span></button></li>)}</ul>
    {lanes && total > lanes.length && <Button variant="outline" disabled={busy} onClick={() => { setBusy(true); void listLanes(token, project, undefined, lanes.length).then(page => { if (active.current) { setLanes(current => [...(current ?? []), ...page.items.filter(item => !current?.some(old => old.id === item.id))]); setTotal(page.total) } }).catch(fail).finally(() => { if (active.current) setBusy(false) }) }}>{t.loadMore}</Button>}
  </div>
}

function LaneDetail({ token, id, project, lang, canWrite, canSend, onAuthError, onBack }: Props & { id: string; onBack: () => void }) {
  const t = pick(STR, lang)
  const [lane, setLane] = useState<Lane | null>(null)
  const [handoffs, setHandoffs] = useState<Handoff[]>([])
  const [total, setTotal] = useState(0)
  const [tickets, setTickets] = useState<Ticket[]>([])
  const [ticketError, setTicketError] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)
  const [refresh, setRefresh] = useState(0)
  const [query, setQuery] = useState('')
  const [draft, setDraft] = useState<HandoffDraft | null>(null)
  const active = useRef(true)
  useEffect(() => { active.current = true; return () => { active.current = false } }, [])
  const fail = useCallback((e: unknown) => { if (!active.current) return; if (isAuthError(e)) onAuthError(); else setError(e instanceof Error ? e.message : String(e)) }, [onAuthError])
  useEffect(() => {
    const controller = new AbortController()
    let timer: ReturnType<typeof setTimeout> | undefined
    async function load() {
      try {
        const [next, history] = await Promise.all([getLane(token, id, controller.signal), listHandoffs(token, id, controller.signal)])
        if (controller.signal.aborted) return
        setLane(next); setHandoffs(history.items); setTotal(history.total); setError('')
        if (history.items.some(h => h.status === 'queued' || h.status === 'running')) timer = setTimeout(() => { void load() }, 4000)
      } catch (e) { if (!controller.signal.aborted) fail(e) }
    }
    void load()
    return () => { controller.abort(); clearTimeout(timer) }
  }, [token, id, refresh, fail])
  useEffect(() => { let cancelled = false; void listTickets(token, project).then(items => { if (!cancelled) { setTickets(items); setTicketError('') } }).catch(e => { if (!cancelled) { if (isAuthError(e)) onAuthError(); else setTicketError(e instanceof Error ? e.message : String(e)) } }); return () => { cancelled = true } }, [token, project, onAuthError])
  async function run(work: () => Promise<unknown>, done?: () => void) {
    setBusy(true); setError('')
    try { await work(); if (active.current) { done?.(); setRefresh(n => n + 1) }; return true } catch (e) { fail(e); return false } finally { if (active.current) setBusy(false) }
  }
  const writable = canWrite && !lane?.archived
  const start = (kind: HandoffDraft['kind'], parent?: Handoff) => setDraft({ kind, provider: parent?.provider ?? 'codex', instructions: kind === 'preparation' ? t.preparing : parent?.kind === 'review' ? `${t.review} ${parent.id} (${parent.target_revision ?? '—'}):\n${parent.result ?? ''}` : '', ticket_ids: parent?.ticket_ids ?? lane?.tickets.map(ticket => ticket.id) ?? [], ...(kind === 'review' && parent ? { parent_handoff: parent.id, target_revision: parent.revision ?? '' } : {}) })
  return <div className="mx-auto max-w-5xl space-y-5">
    <div className="flex flex-wrap justify-between gap-2"><Button variant="ghost" onClick={onBack}>← {t.back}</Button><Button variant="outline" disabled={busy} onClick={() => setRefresh(n => n + 1)}>{t.refresh}</Button></div>
    {error && <p role="alert" className="text-destructive break-words text-sm">{error}</p>}
    {!lane ? !error && <p role="status">{t.loading}</p> : <>
      <h1 className="break-words text-xl font-semibold">{lane.title}</h1>{!writable && <p className="text-muted-foreground text-sm">{t.readOnly}</p>}
      <LaneEditor lane={lane} writable={writable} busy={busy} t={t} onSave={body => run(async () => { const updated = await updateLane(token, id, body); if (active.current) setLane(updated) })} />
      <section className="space-y-3"><h2 className="font-semibold">{t.tickets} <span className="text-muted-foreground text-sm">{lane.tickets.length}</span></h2>{ticketError && <p role="alert" className="text-destructive break-words text-sm">{ticketError}</p>}{!lane.tickets.length && <p className="text-muted-foreground text-sm">{t.noTickets}</p>}
        <ul className="divide-y">{lane.tickets.map(ticket => <li key={ticket.id} className="flex min-w-0 items-center gap-2 py-2"><a className="min-w-0 flex-1 break-words text-sm underline" href={`/board#t=${encodeURIComponent(ticket.id)}`}>{ticket.title || ticket.id}</a><span className="text-muted-foreground text-xs">{ticket.state}</span>{writable && <Button variant="ghost" size="sm" disabled={busy} aria-label={`${t.remove} ${ticket.title}`} onClick={() => void run(() => setLaneTicket(token, id, ticket.id, false))}>{t.remove}</Button>}</li>)}</ul>
        {writable && <div className="space-y-2"><Input aria-label={t.search} placeholder={t.search} value={query} onChange={e => setQuery(e.target.value)} />{query.trim() && <ul className="max-h-52 overflow-y-auto rounded-lg border">{tickets.filter(ticket => !lane.tickets.some(member => member.id === ticket.id) && `${ticket.title} ${ticket.id}`.toLowerCase().includes(query.toLowerCase())).slice(0, 30).map(ticket => <li key={ticket.id}><button className="hover:bg-muted w-full break-words p-2 text-left text-sm" disabled={busy} onClick={() => void run(() => setLaneTicket(token, id, ticket.id, true), () => setQuery(''))}>{t.add}: {ticket.title}</button></li>)}</ul>}</div>}
      </section>
      <section className="space-y-3"><div className="flex flex-wrap items-center justify-between gap-2"><h2 className="font-semibold">{t.handoffs}</h2>{writable && <div className="flex flex-wrap gap-2"><Button variant="outline" onClick={() => start('preparation')}>{t.preparation}</Button><Button onClick={() => start('implementation')}>{t.newHandoff}</Button></div>}</div><p className="text-muted-foreground text-sm">{t.history}</p>
        {draft && writable && <DraftForm key={`${draft.kind}:${draft.parent_handoff ?? ''}`} {...{ draft, lane, handoffs, t, busy }} onCancel={() => setDraft(null)} onSave={value => void run(() => createHandoff(token, id, value), () => setDraft(null))} />}
        {!handoffs.length && <p className="text-muted-foreground text-sm">{t.noHandoffs}</p>}
        {handoffs.map(h => <article key={h.id} className="bg-card min-w-0 space-y-3 rounded-xl border p-4"><div className="flex flex-wrap items-center gap-2"><h3 className="font-medium">{t[h.kind]} · {h.provider === 'codex' ? 'Codex' : 'Claude'}</h3><span className="bg-muted rounded-full px-2 py-0.5 text-xs">{t[h.status]}</span></div><p className="whitespace-pre-wrap break-words text-sm">{h.instructions}</p>
          <div className="text-muted-foreground break-words text-xs">{t.selected}: {h.snapshot.tickets.map(ticket => ticket.title || ticket.id).join(', ') || '—'}{h.target_revision && <p>{t.revision}: {h.target_revision}</p>}</div><details className="text-sm"><summary className="cursor-pointer">{t.snapshot}</summary><div className="mt-2 whitespace-pre-wrap break-words">{h.snapshot.lane.purpose}{'\n'}{h.snapshot.lane.context}</div>{h.snapshot.tickets.map(ticket => <div key={ticket.id} className="mt-3 min-w-0 space-y-2 border-t pt-3"><h4 className="break-words font-medium">{ticket.title || ticket.id} <span className="text-muted-foreground text-xs">{ticket.id} · {ticket.state}</span></h4><Markdown text={ticket.body} />{ticket.links && <dl className="space-y-1 text-xs">{Object.entries(ticket.links).map(([key, value]) => <div key={key} className="break-all"><dt className="font-medium">{key}</dt><dd>{value}</dd></div>)}</dl>}{ticket.metadata != null && <pre className="max-w-full overflow-auto whitespace-pre-wrap break-all text-xs">{JSON.stringify(ticket.metadata, null, 2)}</pre>}</div>)}</details>
          {h.context_applied === false && h.kind === 'preparation' && h.status === 'completed' && <p className="text-amber-600 text-sm">{t.unapplied}</p>}{h.result && <div className="min-w-0 break-words"><h4 className="mb-2 text-sm font-medium">{t.result}</h4><Markdown text={h.result} /></div>}{h.revision && <p className="text-muted-foreground break-all text-xs">{t.revision}: {h.revision}</p>}
          {writable && <div className="flex flex-wrap gap-2">{h.status === 'draft' && <Button disabled={busy || !canSend} title={!canSend ? t.sendPermission : undefined} onClick={() => void run(() => dispatchHandoff(token, h.id))}>{t.send}</Button>}{['draft', 'queued', 'running'].includes(h.status) && <Button variant="outline" disabled={busy || !canSend} title={!canSend ? t.sendPermission : undefined} onClick={() => void run(() => cancelHandoff(token, h.id))}>{t.cancel}</Button>}{h.status === 'completed' && h.kind === 'implementation' && h.revision && <Button variant="outline" onClick={() => start('review', h)}>{t.reviewAction}</Button>}{h.status === 'completed' && h.kind === 'review' && <Button variant="outline" onClick={() => start('implementation', h)}>{t.fixAction}</Button>}</div>}
        </article>)}{total > handoffs.length && <Button variant="outline" disabled={busy} onClick={() => { setBusy(true); void listHandoffs(token, id, undefined, handoffs.length).then(page => { if (active.current) { setHandoffs(current => [...current, ...page.items.filter(item => !current.some(old => old.id === item.id))]); setTotal(page.total) } }).catch(fail).finally(() => { if (active.current) setBusy(false) }) }}>{t.loadMore}</Button>}
      </section>
    </>}
  </div>
}

function DraftForm({ draft, lane, handoffs, t, busy, onSave, onCancel }: { draft: HandoffDraft; lane: Lane; handoffs: Handoff[]; t: Labels; busy: boolean; onSave: (draft: HandoffDraft) => void; onCancel: () => void }) {
  const [value, setValue] = useState(draft)
  return <form className="space-y-3 rounded-xl border p-4" onSubmit={e => { e.preventDefault(); onSave(value) }}><h3 className="font-medium">{t[value.kind]}</h3><p className="text-muted-foreground text-sm">{t.draftHint}</p>
    <label className="block text-sm">{t.provider}<select className={selectClass} value={value.provider} onChange={e => setValue({ ...value, provider: e.target.value as Provider })}><option value="codex">Codex</option><option value="claude">Claude</option></select></label><label className="block text-sm">{t.instructions}<Textarea required value={value.instructions} onChange={e => setValue({ ...value, instructions: e.target.value })} /></label>
    {value.kind === 'review' && <><label className="block text-sm">{t.parent}<select required className={selectClass} value={value.parent_handoff ?? ''} onChange={e => { const parent = handoffs.find(h => h.id === e.target.value); setValue({ ...value, parent_handoff: e.target.value, target_revision: parent?.revision ?? '' }) }}><option value="">{t.choose}</option>{handoffs.filter(h => h.kind === 'implementation' && h.status === 'completed' && h.revision).map(h => <option key={h.id} value={h.id}>{h.id} · {h.revision ?? h.instructions.slice(0, 60)}</option>)}</select></label><label className="block text-sm">{t.revision}<Input required readOnly value={value.target_revision ?? ''} onChange={e => setValue({ ...value, target_revision: e.target.value })} /></label></>}
    <fieldset className="space-y-2"><legend className="mb-2 text-sm">{t.selectTickets}</legend>{lane.tickets.map(ticket => <label key={ticket.id} className="flex min-w-0 items-start gap-2 text-sm"><input type="checkbox" checked={value.ticket_ids.includes(ticket.id)} onChange={e => setValue({ ...value, ticket_ids: e.target.checked ? [...value.ticket_ids, ticket.id] : value.ticket_ids.filter(id => id !== ticket.id) })} /><span className="break-words">{ticket.title || ticket.id}</span></label>)}</fieldset>
    <div className="flex flex-wrap gap-2"><Button disabled={busy || !value.instructions.trim() || (value.kind !== 'preparation' && value.ticket_ids.length === 0)}>{t.saveDraft}</Button><Button type="button" variant="ghost" onClick={onCancel}>{t.cancel}</Button></div>
  </form>
}

function LaneEditor({ lane, writable, busy, t, onSave }: { lane: Lane; writable: boolean; busy: boolean; t: Labels; onSave: (body: { title: string; purpose: string; context: string }) => Promise<boolean> }) {
  const [value, setValue] = useState({ title: lane.title, purpose: lane.purpose, context: lane.context })
  const [dirty, setDirty] = useState(false)
  useEffect(() => { if (!dirty) setValue({ title: lane.title, purpose: lane.purpose, context: lane.context }) }, [lane.title, lane.purpose, lane.context, dirty])
  return <form className="space-y-3" onSubmit={e => { e.preventDefault(); void onSave(value).then(saved => { if (saved) setDirty(false) }) }}>
    <label className="block text-sm">{t.laneTitle}<Input value={value.title} required disabled={!writable || busy} onChange={e => { setDirty(true); setValue({ ...value, title: e.target.value }) }} /></label>
    <label className="block text-sm">{t.purpose}<Textarea value={value.purpose} disabled={!writable || busy} onChange={e => { setDirty(true); setValue({ ...value, purpose: e.target.value }) }} /></label>
    <label className="block text-sm">{t.context}<Textarea value={value.context} disabled={!writable || busy} onChange={e => { setDirty(true); setValue({ ...value, context: e.target.value }) }} /></label>
    {writable && <div className="flex gap-2"><Button disabled={busy || !value.title.trim()}>{t.save}</Button>{dirty && <Button type="button" variant="ghost" onClick={() => setDirty(false)}>{t.cancel}</Button>}</div>}
  </form>
}
