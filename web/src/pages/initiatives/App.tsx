// /initiatives — the one SPA that WRITES.
//
// Initiatives are not work: no workflow, no claim, no lease, no ready queue.
// `status` is a label. Everything accumulates through append-only entries, each
// recording where it came from, which is what makes the collection weighable
// later instead of an undifferentiated pile of text.
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'

import { AppHeader } from '@/components/AppHeader'
import { useNavigate } from 'react-router'
import { loadToken, saveToken } from '@/lib/session'
import { EditableText } from '@/components/EditableText'
import { TokenGate } from '@/components/TokenGate'
import { useToast } from '@/components/Toaster'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Composer, type Draft, type PickedFile } from '@/components/initiatives/Composer'
import { CreateDialog } from '@/components/initiatives/CreateDialog'
import { EntryCard } from '@/components/initiatives/EntryCard'
import { InitiativeRow } from '@/components/initiatives/InitiativeRow'
import { RollupStrip } from '@/components/initiatives/RollupStrip'

import { detectLocale, pick, type Locale } from '@/lib/i18n'
import { localInputToRfc3339 } from '@/lib/format'
import { cn } from '@/lib/utils'
import {
  STATUSES,
  appendEntry,
  createInitiative,
  downloadAttachment,
  listEntries,
  listInitiatives,
  listProjects,
  patchInitiative,
  readFileAsBase64,
  whoami,
  type CreateFields,
  type Entry,
  type Initiative,
  type InitiativeStatus,
  type Project,
} from '@/lib/initiatives'
import { STR } from './strings'

const LS_LANG = 'takomo.lang'
const LS_PROJECT = 'takomo.initiatives.project'

interface Me {
  actor: string
  scopes: string[]
}

const EMPTY_DRAFT: Draft = { kind: 'note', source: '', title: '', text: '', uri: '', origin: '' }

export function App() {
  const navigate = useNavigate()
  const { toast } = useToast()

  const [token, setToken] = useState(() => loadToken())
  const [lang, setLang] = useState<Locale>(() => detectLocale(localStorage.getItem(LS_LANG)))
  const [project, setProject] = useState(() => localStorage.getItem(LS_PROJECT) ?? '')
  const [gateError, setGateError] = useState('')

  const [me, setMe] = useState<Me>({ actor: '', scopes: [] })
  const [projects, setProjects] = useState<Project[]>([])
  const [items, setItems] = useState<Initiative[]>([])
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [entries, setEntries] = useState<Entry[]>([])
  const [entryCursor, setEntryCursor] = useState<string | null>(null)

  const [status, setStatus] = useState('')
  const [q, setQ] = useState('')
  const [creating, setCreating] = useState(false)
  const [busy, setBusy] = useState(false)
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT)
  const [file, setFile] = useState<PickedFile | null>(null)

  const t = useMemo(() => pick(STR, lang), [lang])
  const canWrite = me.scopes.includes('write')

  const statusLabel = useCallback(
    (s: InitiativeStatus) =>
      s === 'parked' ? t.statusParked : s === 'distilled' ? t.statusDistilled : t.statusOpen,
    [t],
  )

  // A rejected token drops straight back to the gate; anything else is a toast
  // carrying the API's own message and remedy, verbatim.
  const handleErr = useCallback(
    (e: unknown) => {
      const err = e as { auth?: boolean; message?: string }
      if (err?.auth) {
        setToken('')
        saveToken('')
        setGateError('')
        return
      }
      toast(err?.message || t.requestFailed, 'err')
    },
    [toast, t],
  )

  // ---- fetching -----------------------------------------------------------

  const fetchEntries = useCallback(
    async (id: string, replace: boolean, cursor?: string | null) => {
      const page = await listEntries(token, id, replace ? null : cursor)
      setEntries((prev) => (replace ? page.items : [...prev, ...page.items]))
      setEntryCursor(page.next_cursor ?? null)
      // The entry list carries the whole collection's rollup, so the selected
      // initiative's counts refresh from the same response.
      if (page.rollup) {
        setItems((prev) => prev.map((i) => (i.id === id ? { ...i, rollup: page.rollup } : i)))
      }
    },
    [token],
  )

  const fetchAll = useCallback(async () => {
    const page = await listInitiatives(token, { project, status, q })
    const next = page.items ?? []
    setItems(next)
    // Keep the selection only while it is still in view.
    setSelectedId((cur) => {
      if (cur && !next.some((i) => i.id === cur)) {
        setEntries([])
        setEntryCursor(null)
        return null
      }
      return cur
    })
  }, [token, project, status, q])

  const select = useCallback(
    (id: string) => {
      setSelectedId(id)
      setEntries([])
      setEntryCursor(null)
      window.location.hash = 'i=' + id
      fetchEntries(id, true).catch(handleErr)
    },
    [fetchEntries, handleErr],
  )

  // Boot: projects + who am I, then the list. Re-runs when the token changes,
  // which is exactly what signing in should do.
  useEffect(() => {
    if (!token) return
    let cancelled = false
    ;(async () => {
      try {
        const [ps, who] = await Promise.all([
          listProjects(token).catch(() => [] as Project[]),
          whoami(token).catch(() => ({ actor: '', scopes: [] })),
        ])
        if (cancelled) return
        setProjects(ps)
        setMe({ actor: who.actor ?? '', scopes: who.scopes ?? [] })
        // The composer's source defaults to who you are: the common case is that
        // the person reading is the person the input came from.
        setDraft((d) => (d.source ? d : { ...d, source: who.actor ?? '' }))
      } catch (e) {
        if (!cancelled) handleErr(e)
      }
    })()
    return () => {
      cancelled = true
    }
  }, [token, handleErr])

  // The list, refetched whenever a filter changes. The search box is debounced
  // by the caller, so this fires once the typing settles.
  useEffect(() => {
    if (!token) return
    fetchAll().catch(handleErr)
  }, [token, fetchAll, handleErr])

  // ---- hash routing (#i=ini-…) -------------------------------------------

  // Refs, not deps: this listener must be installed exactly once, and a state
  // updater must stay pure — calling `select()` from inside one would fire twice
  // under StrictMode and issue two entry fetches.
  const selectRef = useRef(select)
  selectRef.current = select
  const selectedIdRef = useRef<string | null>(null)
  selectedIdRef.current = selectedId

  useEffect(() => {
    function apply() {
      const m = /(?:^#|&)i=([^&]+)/.exec(window.location.hash || '')
      const id = m?.[1] ? decodeURIComponent(m[1]) : null
      // Comparing against the current selection is also what makes `select()`
      // writing the hash harmless: the resulting hashchange finds nothing to do.
      if (id && id !== selectedIdRef.current) selectRef.current(id)
    }
    apply()
    window.addEventListener('hashchange', apply)
    return () => window.removeEventListener('hashchange', apply)
  }, [])

  // ---- writes -------------------------------------------------------------

  const guardWrite = useCallback(() => {
    if (canWrite) return true
    toast(t.writeNeeded, 'err')
    return false
  }, [canWrite, toast, t])

  const doPatch = useCallback(
    async (id: string, body: Parameters<typeof patchInitiative>[2], note: string) => {
      const updated = await patchInitiative(token, id, body)
      setItems((prev) => prev.map((i) => (i.id === id ? updated : i)))
      toast(note, 'success')
      return updated
    },
    [token, toast],
  )

  const doCreate = useCallback(
    async (fields: CreateFields) => {
      try {
        const ini = await createInitiative(token, fields)
        setCreating(false)
        toast(t.created, 'success')
        await fetchAll()
        select(ini.id)
      } catch (e) {
        handleErr(e)
      }
    },
    [token, toast, t, fetchAll, select, handleErr],
  )

  const doAppend = useCallback(async () => {
    if (!selectedId || !guardWrite()) return
    if (!draft.source.trim()) {
      toast(t.needSource, 'err')
      return
    }
    if (!draft.text.trim() && !file) {
      toast(t.needBody, 'err')
      return
    }
    const body: Parameters<typeof appendEntry>[2] = {
      kind: (draft.kind || 'note').trim(),
      source: draft.source.trim(),
    }
    if (draft.text.trim()) body.text = draft.text
    if (draft.title.trim()) body.title = draft.title.trim()
    if (draft.uri.trim()) body.source_uri = draft.uri.trim()
    const origin = localInputToRfc3339(draft.origin)
    if (origin) body.origin_at = origin
    if (file) {
      body.content_base64 = file.b64
      body.filename = file.name
      if (file.mime) body.mime = file.mime
    }

    setBusy(true)
    try {
      await appendEntry(token, selectedId, body)
      // Kind and source persist between entries — they are usually the same for
      // a run of them — everything else clears.
      setDraft((d) => ({ ...EMPTY_DRAFT, kind: d.kind, source: d.source }))
      setFile(null)
      toast(t.appended, 'success')
      await fetchEntries(selectedId, true)
    } catch (e) {
      handleErr(e)
    } finally {
      setBusy(false)
    }
  }, [selectedId, guardWrite, draft, file, token, toast, t, fetchEntries, handleErr])

  const onPickFile = useCallback(
    (f: File | null) => {
      if (!f) {
        setFile(null)
        return
      }
      readFileAsBase64(f)
        .then(setFile)
        .catch(() => toast(t.fileUnreadable, 'err'))
    },
    [toast, t],
  )

  // ---- gate ---------------------------------------------------------------

  function signIn(tk: string) {
    saveToken(tk)
    setGateError('')
    setToken(tk)
  }

  function signOut() {
    saveToken('')
    setToken('')
    setItems([])
    setSelectedId(null)
    setEntries([])
  }

  if (!token) {
    return (
      <TokenGate
        title="takomo · initiatives"
        subtitle={t.gateTokenSub}
        tokenLabel={t.gateLabel}
        openLabel={t.gateOpen}
        emptyMessage={t.tokenNeeded}
        error={gateError}
        onSubmit={signIn}
      />
    )
  }

  const selected = items.find((i) => i.id === selectedId) ?? null

  return (
    <div className="flex h-screen flex-col overflow-hidden">
      <AppHeader
        onNavigate={navigate}
        current="initiatives"
        nav={{
          board: t.board,
          inbox: t.inbox,
          initiatives: t.initiatives,
          schedules: t.schedules,
        }}
        lang={lang}
        onLang={(l) => {
          setLang(l)
          localStorage.setItem(LS_LANG, l)
        }}
        projects={projects.map((p) => ({ id: p.id }))}
        project={project}
        allProjectsLabel={t.allProjects}
        projectLabel={t.project}
        onProject={(id) => {
          setProject(id)
          localStorage.setItem(LS_PROJECT, id)
          setSelectedId(null)
          setEntries([])
          setEntryCursor(null)
        }}
      >
        <Button
          onClick={() => {
            if (!guardWrite()) return
            if (!project) {
              toast(t.needProject, 'err')
              return
            }
            setCreating(true)
          }}
        >
          + {t.newInitiative}
        </Button>
        <Button variant="outline" size="icon" title={t.refresh} onClick={() => fetchAll().catch(handleErr)}>
          ↻
        </Button>
        <Button variant="outline" size="icon" title={t.signOut} onClick={signOut}>
          ⎋
        </Button>
      </AppHeader>

      <main className="grid min-h-0 flex-1 grid-cols-1 grid-rows-[auto_auto_1fr] md:grid-cols-[380px_1fr] md:grid-rows-[auto_1fr]">
        <div className="bg-card border-b-border-soft col-span-full flex flex-wrap items-center gap-2 border-b px-4.5 py-2.25">
          {/* Deliberately no counts on these chips. The list is fetched already
              filtered, so a count derived from it would read 0 on every chip but
              the active one — a number that looks authoritative and is wrong. */}
          <FilterChip label={t.all} on={status === ''} onClick={() => setStatus('')} />
          {STATUSES.map((s) => (
            <FilterChip
              key={s}
              label={statusLabel(s)}
              on={status === s}
              onClick={() => setStatus(s)}
            />
          ))}
          <SearchBox placeholder={t.searchPh} value={q} onChange={setQ} />
        </div>

        <section className="bg-card border-r-border-soft max-h-[34vh] min-h-0 overflow-y-auto border-b md:max-h-none md:border-r md:border-b-0">
          {items.length === 0 ? (
            <Empty big={t.emptyList} hint={t.emptyListHint} />
          ) : (
            items.map((i) => (
              <InitiativeRow
                key={i.id}
                initiative={i}
                selected={i.id === selectedId}
                statusLabel={statusLabel(i.status)}
                entriesWord={t.rEntries.toLowerCase()}
                onSelect={select}
              />
            ))
          )}
        </section>

        <section className="min-h-0 overflow-y-auto">
          {!selected ? (
            <Empty big={t.noneSelected} hint={t.noneSelectedHint} />
          ) : (
            <div className="max-w-215 px-6.5 pt-5 pb-10">
              <div className="flex items-start gap-3">
                <EditableText
                  key={selected.id + ':title'}
                  as="h1"
                  value={selected.title}
                  editable={canWrite}
                  required
                  className={cn(
                    'm-0 grow text-[22px] font-[740] tracking-[-0.02em] outline-none',
                    canWrite && 'focus:border-ring border-b border-dashed border-transparent',
                  )}
                  onCommit={(next) => doPatch(selected.id, { title: next }, t.savedTitle)}
                />
                <select
                  aria-label={t.aStatus}
                  value={selected.status}
                  disabled={!canWrite}
                  onChange={(e) =>
                    doPatch(
                      selected.id,
                      { status: e.target.value as InitiativeStatus },
                      t.savedStatus,
                    ).catch(handleErr)
                  }
                  className="bg-secondary text-secondary-foreground border-ring cursor-pointer appearance-none rounded-md border px-2.25 py-1 text-[11.5px] font-bold tracking-[0.04em] uppercase"
                >
                  {STATUSES.map((s) => (
                    <option key={s} value={s}>
                      {statusLabel(s)}
                    </option>
                  ))}
                </select>
              </div>

              <div className="text-muted-foreground font-mono text-[11.5px]">
                {selected.id} · {selected.project}
              </div>

              <EditableText
                key={selected.id + ':summary'}
                value={selected.summary ?? ''}
                editable={canWrite}
                className={cn(
                  'text-foreground mt-2.25 mb-0 min-h-5 text-[14px] outline-none',
                  canWrite && 'focus:border-ring border-b border-dashed border-transparent',
                )}
                onCommit={(next) => doPatch(selected.id, { summary: next }, t.savedSummary)}
              />

              <RollupStrip
                rollup={selected.rollup}
                labels={{
                  entries: t.rEntries,
                  attachments: t.rAttachments,
                  chars: t.rChars,
                  size: t.rSize,
                  last: t.rLast,
                }}
              />

              {((selected.labels?.length ?? 0) > 0 || (selected.tags?.length ?? 0) > 0) && (
                <div className="mt-2.5 flex flex-wrap gap-1.5">
                  {selected.labels?.map((l) => (
                    <span
                      key={l}
                      className="bg-muted border-border text-foreground rounded-[5px] border px-1.75 py-0.5 text-[11.5px]"
                    >
                      {l}
                    </span>
                  ))}
                  {selected.tags?.map((tg) => (
                    <span
                      key={tg}
                      className="bg-secondary text-secondary-foreground rounded-[5px] px-1.75 py-0.5 font-mono text-[11.5px]"
                    >
                      {tg}
                    </span>
                  ))}
                </div>
              )}

              {canWrite && (
                <>
                  <SectionHeader>{t.addEntryHdr}</SectionHeader>
                  <Composer
                    draft={draft}
                    onDraft={(patch) => setDraft((d) => ({ ...d, ...patch }))}
                    file={file}
                    onPickFile={onPickFile}
                    busy={busy}
                    onAppend={doAppend}
                    labels={{
                      kind: t.eKind,
                      kindHint: t.eKindHint,
                      source: t.eSource,
                      sourceHint: t.eSourceHint,
                      title: t.eTitle,
                      titlePh: t.eTitlePh,
                      uri: t.eSourceUri,
                      uriPh: t.eSourceUriPh,
                      text: t.eText,
                      textPh: t.eTextPh,
                      origin: t.eOrigin,
                      originHint: t.eOriginHint,
                      attach: t.attach,
                      attachClear: t.attachClear,
                      attachAria: t.aAttach,
                      append: t.append,
                      appending: t.appending,
                    }}
                  />
                </>
              )}

              <SectionHeader>{t.entriesHdr}</SectionHeader>
              {entries.length === 0 ? (
                <Empty big={t.emptyEntries} hint={t.emptyEntriesHint} />
              ) : (
                <>
                  {entries.map((en) => (
                    <EntryCard
                      key={en.id}
                      entry={en}
                      labels={{ by: t.by, wrote: t.wrote, download: t.download }}
                      onDownload={(e) => downloadAttachment(token, e).catch(handleErr)}
                    />
                  ))}
                  {entryCursor && (
                    <Button
                      variant="outline"
                      onClick={() =>
                        fetchEntries(selected.id, false, entryCursor).catch(handleErr)
                      }
                    >
                      {t.loadMore}
                    </Button>
                  )}
                </>
              )}
            </div>
          )}
        </section>
      </main>

      <CreateDialog
        open={creating}
        onOpenChange={setCreating}
        project={project}
        onCreate={doCreate}
        onInvalid={(m) => toast(m, 'err')}
        labels={{
          title: t.newInitiative,
          subtitle: t.newSub,
          fTitle: t.fTitle,
          fTitlePh: t.fTitlePh,
          fSummary: t.fSummary,
          fSummaryPh: t.fSummaryPh,
          fLabels: t.fLabels,
          fLabelsPh: t.fLabelsPh,
          fTags: t.fTags,
          fTagsPh: t.fTagsPh,
          create: t.create,
          cancel: t.cancel,
          needTitle: t.needTitle,
        }}
      />
    </div>
  )
}

function FilterChip({ label, on, onClick }: { label: string; on: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        'border-border text-muted-foreground bg-muted hover:text-primary hover:border-ring cursor-pointer rounded-full border px-3 py-1.25 text-[12.5px] font-[650]',
        on && 'bg-secondary border-ring text-secondary-foreground',
      )}
    >
      {label}
    </button>
  )
}

// Typing filters the server-side list, so each keystroke would be a request.
// 250 ms is what the page used and it is the right trade: fast enough to feel
// live, slow enough that a word is one query.
function SearchBox({
  placeholder,
  value,
  onChange,
}: {
  placeholder: string
  value: string
  onChange: (v: string) => void
}) {
  const [local, setLocal] = useState(value)
  useEffect(() => {
    const id = setTimeout(() => onChange(local), 250)
    return () => clearTimeout(id)
  }, [local, onChange])
  return (
    <Input
      type="search"
      aria-label={placeholder}
      placeholder={placeholder}
      value={local}
      onChange={(e) => setLocal(e.target.value)}
      className="w-55"
    />
  )
}

function SectionHeader({ children }: { children: React.ReactNode }) {
  return (
    <div className="text-muted-foreground mt-6.5 mb-2.5 text-[11.5px] font-[750] tracking-[0.06em] uppercase">
      {children}
    </div>
  )
}

function Empty({ big, hint }: { big: string; hint: string }) {
  return (
    <div className="text-muted-foreground px-6.5 py-15 text-center">
      <div className="text-foreground mb-1.5 text-[15px] font-[680]">{big}</div>
      <div>{hint}</div>
    </div>
  )
}
