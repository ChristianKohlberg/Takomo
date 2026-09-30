import { useCallback, useEffect, useRef, useState } from 'react'
import { ChevronRight, CircleCheck, CircleX, Copy, Link2, Pencil, UserCheck, X } from 'lucide-react'
import { Markdown } from '@/components/Markdown'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import {
  deleteBehavior,
  describeTest,
  getBehavior,
  manualKey,
  patchBehavior,
  reportRun,
  type BehaviorDetail as Detail,
  type BehaviorStatus,
  type HistoryEntry,
  type TestKind,
  type TestResult,
} from '@/lib/behaviors'
import { fmtAge } from '@/lib/format'
import type { PlanNode } from '@/lib/plan-sections'
import { cn } from '@/lib/utils'
import { SectionSelect } from './SectionSelect'
import { StatusIcon } from './status'
import type { STR } from './strings'

type Labels = (typeof STR)['en']
const DAY_MS = 86_400_000

export function kindLabel(kind: TestKind, t: Labels): string {
  switch (kind) {
    case 'browser':
      return t.kindBrowser
    case 'component':
      return t.kindComponent
    case 'unit':
      return t.kindUnit
    case 'integration':
      return t.kindIntegration
    case 'contract':
      return t.kindContract
    case 'agent':
      return t.kindAgent
    case 'manual':
      return t.kindManual
    case 'other':
      return t.kindOther
  }
}

/** One test's own status, by the same rules as a behavior's, so the tree explains the headline. */
function testStatus(item: TestResult, freshDays: number, now: number): BehaviorStatus {
  if (!item.latest) return 'untested'
  if (item.latest.outcome === 'fail') return 'failing'
  return now - new Date(item.latest.at).getTime() <= freshDays * DAY_MS ? 'verified' : 'stale'
}

/**
 * One promise, opened in place under its row. What it promises comes first,
 * then the evidence that shows it and its track record; checking it by hand,
 * the edit form, every check and the raw test keys wait behind a click.
 */
export function BehaviorDetail({
  token,
  project,
  id,
  nodes,
  canWrite,
  t,
  freshDays,
  version,
  onChanged,
  onDeleted,
  onError,
}: {
  token: string
  project: string
  id: string
  nodes: PlanNode[]
  canWrite: boolean
  t: Labels
  freshDays: number
  /** Bumped by the parent when a live update may have changed this behavior. */
  version: number
  onChanged: () => void
  onDeleted: () => void
  onError: (error: unknown) => void
}) {
  const [detail, setDetail] = useState<Detail | null>(null)
  const [missing, setMissing] = useState('')
  const [editing, setEditing] = useState(false)
  const [title, setTitle] = useState('')
  const [statement, setStatement] = useState('')
  const [section, setSection] = useState<string | null>(null)
  const [linking, setLinking] = useState(false)
  const [newTest, setNewTest] = useState('')
  const [showHistory, setShowHistory] = useState(false)
  const [showTechnical, setShowTechnical] = useState(false)
  const [copied, setCopied] = useState('')
  const [checking, setChecking] = useState(false)
  const [checkText, setCheckText] = useState('')
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const epoch = useRef(0)

  const load = useCallback(async () => {
    const attempt = ++epoch.current
    try {
      const next = await getBehavior(token, id)
      if (attempt !== epoch.current) return
      setDetail(next)
      setMissing('')
    } catch (error) {
      if (attempt !== epoch.current) return
      const failure = error as { status?: number; auth?: boolean; message?: string }
      if (failure.auth) onError(error)
      setMissing(failure.message || String(error))
    }
  }, [token, id, onError])

  useEffect(() => {
    const counter = epoch
    void load()
    return () => {
      counter.current++
    }
  }, [load, version])

  const save = async (fields: Parameters<typeof patchBehavior>[2]): Promise<boolean> => {
    setBusy(true)
    setMessage('')
    try {
      await patchBehavior(token, id, fields)
      await load()
      onChanged()
      return true
    } catch (error) {
      onError(error)
      return false
    } finally {
      setBusy(false)
    }
  }

  if (missing)
    return (
      <p role="alert" className="text-sm break-words">
        {missing}
      </p>
    )
  if (!detail)
    return (
      <p role="status" className="text-muted-foreground text-sm">
        {t.loading}
      </p>
    )

  const now = Date.now()
  const startEditing = () => {
    setTitle(detail.title)
    setStatement(detail.statement)
    setSection(detail.section)
    setMessage('')
    setEditing(true)
  }

  if (editing)
    return (
      <form
        className="grid gap-3"
        aria-label={t.edit}
        onSubmit={(event) => {
          event.preventDefault()
          if (!title.trim()) {
            setMessage(t.titleRequired)
            return
          }
          void save({ title: title.trim(), statement: statement.trim(), section }).then((saved) => {
            if (saved) setEditing(false)
          })
        }}
      >
        <div className="grid gap-1.5">
          <Label htmlFor={`behavior-title-${id}`}>{t.fTitle}</Label>
          <Input id={`behavior-title-${id}`} value={title} onChange={(event) => setTitle(event.target.value)} />
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor={`behavior-statement-${id}`}>{t.fStatement}</Label>
          <Textarea
            id={`behavior-statement-${id}`}
            rows={5}
            value={statement}
            placeholder={t.fStatementPh}
            onChange={(event) => setStatement(event.target.value)}
          />
          <p className="text-muted-foreground m-0 text-xs">{t.fStatementHint}</p>
        </div>
        <div className="grid gap-1.5">
          <Label htmlFor={`behavior-section-${id}`}>{t.fSection}</Label>
          <SectionSelect
            id={`behavior-section-${id}`}
            nodes={nodes}
            value={section}
            noneLabel={t.noSection}
            onChange={setSection}
          />
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button type="submit" size="sm" disabled={busy}>
            {t.save}
          </Button>
          <Button type="button" size="sm" variant="ghost" disabled={busy} onClick={() => setEditing(false)}>
            {t.cancel}
          </Button>
          {message && (
            <span role="status" className="text-muted-foreground text-xs">
              {message}
            </span>
          )}
          <span className="grow" />
          <Button
            type="button"
            variant="ghost"
            size="sm"
            className="text-nf"
            disabled={busy}
            onClick={() => {
              if (!window.confirm(t.confirmDelete)) return
              setBusy(true)
              void deleteBehavior(token, id)
                .then(onDeleted)
                .catch(onError)
                .finally(() => setBusy(false))
            }}
          >
            {t.delete}
          </Button>
        </div>
      </form>
    )

  const addTest = () => {
    const key = newTest.trim()
    if (!key || detail.tests.includes(key)) return
    // Clear the field only once the key is saved, so a failed save keeps it.
    void save({ add_tests: [key] }).then((saved) => {
      if (saved) {
        setNewTest('')
        setLinking(false)
      }
    })
  }

  // A check by hand is a run like any other, under this promise's own manual
  // key, so the status, the track record and the freshness rule stay one thing.
  const recordCheck = async (outcome: 'pass' | 'fail') => {
    const key = manualKey(id)
    setBusy(true)
    setMessage('')
    try {
      if (!detail.tests.includes(key)) await patchBehavior(token, id, { add_tests: [key] })
      const detailText = checkText.trim()
      await reportRun(token, project, {
        note: t.checkNote,
        results: [{ test: key, outcome, ...(detailText ? { detail: detailText } : {}) }],
      })
      setChecking(false)
      setCheckText('')
      setMessage(t.checkSaved)
      await load()
      onChanged()
    } catch (error) {
      onError(error)
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="grid min-w-0 gap-5">
      {detail.statement && (
        <section className="grid gap-1" aria-label={t.promise}>
          <h3 className="text-muted-foreground m-0 text-[11px] font-semibold tracking-wider uppercase">{t.promise}</h3>
          <Markdown text={detail.statement} className="max-w-[68ch] text-sm" />
        </section>
      )}

      <section className="grid gap-2" aria-label={t.shownBy}>
        <h3 className="text-muted-foreground m-0 text-[11px] font-semibold tracking-wider uppercase">{t.shownBy}</h3>
        {detail.test_results.length === 0 ? (
          <p className="text-muted-foreground m-0 text-sm">{t.noTests}</p>
        ) : (
          <ul className="m-0 grid list-none gap-2.5 p-0">
            {detail.test_results.map((item) => {
              const test = describeTest(item.test)
              const name = test.kind === 'manual' ? t.manualName : test.name
              const status = testStatus(item, freshDays, now)
              return (
                <li key={item.test} className="flex min-w-0 items-start gap-2">
                  <StatusIcon status={status} className="mt-0.5" />
                  <div className="min-w-0 flex-1">
                    <p className="m-0 text-sm break-words">{name}</p>
                    <p className="text-muted-foreground m-0 text-xs">
                      {kindLabel(test.kind, t)}
                      {' · '}
                      {item.latest
                        ? `${item.latest.outcome === 'pass' ? t.pass : t.fail} ${t.ago.replace('{age}', fmtAge(item.latest.at, now))}`
                        : t.neverReported}
                    </p>
                    {item.latest?.detail && (
                      <p
                        className={cn(
                          'm-0 mt-1 rounded-md px-2 py-1 text-xs break-words whitespace-pre-wrap',
                          item.latest.outcome === 'fail' ? 'bg-nfbg text-nf' : 'bg-muted text-muted-foreground',
                        )}
                      >
                        {item.latest.detail}
                      </p>
                    )}
                  </div>
                  {canWrite && (
                    <button
                      type="button"
                      aria-label={t.removeTest.replace('{test}', name)}
                      title={t.removeTest.replace('{test}', name)}
                      className="text-muted-foreground hover:text-foreground cursor-pointer rounded p-0.5"
                      disabled={busy}
                      onClick={() => void save({ remove_tests: [item.test] })}
                    >
                      <X className="size-3.5" aria-hidden="true" />
                    </button>
                  )}
                </li>
              )
            })}
          </ul>
        )}
        {linking && (
          <form
            className="flex min-w-0 gap-2"
            onSubmit={(event) => {
              event.preventDefault()
              addTest()
            }}
          >
            <Input
              autoFocus
              aria-label={t.addTestPh}
              placeholder={t.addTestPh}
              value={newTest}
              className="min-w-0 flex-1 font-mono text-xs"
              onChange={(event) => setNewTest(event.target.value)}
            />
            <Button type="submit" size="sm" disabled={busy || !newTest.trim()}>
              {t.addTest}
            </Button>
            <Button type="button" size="sm" variant="ghost" onClick={() => setLinking(false)}>
              {t.cancel}
            </Button>
          </form>
        )}
      </section>

      <TrackRecord entries={detail.history} t={t} now={now} />

      {checking && (
        <form
          className="bg-muted/50 grid gap-2 rounded-lg p-3"
          aria-label={t.checkByHand}
          onSubmit={(event) => event.preventDefault()}
        >
          <p className="m-0 text-sm">{t.checkPrompt}</p>
          <Input
            autoFocus
            aria-label={t.checkNotePh}
            placeholder={t.checkNotePh}
            value={checkText}
            onChange={(event) => setCheckText(event.target.value)}
          />
          <div className="flex flex-wrap gap-2">
            <Button type="button" size="sm" variant="outline" className="text-ok" disabled={busy} onClick={() => void recordCheck('pass')}>
              <CircleCheck className="size-3.5" aria-hidden="true" /> {t.checkWorks}
            </Button>
            <Button type="button" size="sm" variant="outline" className="text-nf" disabled={busy} onClick={() => void recordCheck('fail')}>
              <CircleX className="size-3.5" aria-hidden="true" /> {t.checkBroken}
            </Button>
            <Button type="button" size="sm" variant="ghost" disabled={busy} onClick={() => setChecking(false)}>
              {t.cancel}
            </Button>
          </div>
        </form>
      )}

      <div className="flex flex-wrap items-center gap-x-1 gap-y-2 border-t pt-3">
        {canWrite && (
          <>
            <Button size="sm" variant="outline" onClick={startEditing}>
              <Pencil className="size-3.5" aria-hidden="true" /> {t.edit}
            </Button>
            {!linking && (
              <Button size="sm" variant="ghost" onClick={() => setLinking(true)}>
                <Link2 className="size-3.5" aria-hidden="true" /> {t.linkTest}
              </Button>
            )}
            {!checking && (
              <Button size="sm" variant="ghost" onClick={() => setChecking(true)}>
                <UserCheck className="size-3.5" aria-hidden="true" /> {t.checkByHand}
              </Button>
            )}
          </>
        )}
        {message && (
          <span role="status" className="text-muted-foreground px-2 text-xs">
            {message}
          </span>
        )}
        <span className="grow" />
        <Disclosure open={showHistory} onToggle={() => setShowHistory((v) => !v)} label={t.history} />
        <Disclosure open={showTechnical} onToggle={() => setShowTechnical((v) => !v)} label={t.technical} />
      </div>

      {showHistory && <History entries={detail.history} t={t} now={now} />}

      {showTechnical && (
        <section className="bg-muted grid gap-2 rounded-md p-3 text-xs" aria-label={t.technical}>
          <ul className="m-0 grid list-none gap-1.5 p-0">
            {detail.tests.map((key) => (
              <li key={key} className="flex min-w-0 items-start gap-2">
                <code className="min-w-0 flex-1 break-all">{key}</code>
                <button
                  type="button"
                  className="text-muted-foreground hover:text-foreground inline-flex shrink-0 cursor-pointer items-center gap-1"
                  onClick={() => {
                    void navigator.clipboard?.writeText(key).then(() => setCopied(key))
                  }}
                >
                  <Copy className="size-3.5" aria-hidden="true" /> {copied === key ? t.copied : t.copyKey}
                </button>
              </li>
            ))}
          </ul>
          <p className="text-muted-foreground m-0">
            <code>{detail.id}</code> · {t.createdBy.replace('{actor}', detail.created_by)}
          </p>
          <p className="text-muted-foreground m-0">{t.freshness.replace('{days}', String(freshDays))}</p>
        </section>
      )}
    </div>
  )
}

/** Checks newest first, one per run: a run fails when any of its results failed. */
function runsOf(entries: HistoryEntry[]): { run: string; at: string; outcome: 'pass' | 'fail' }[] {
  const runs: { run: string; at: string; outcome: 'pass' | 'fail' }[] = []
  for (const entry of entries) {
    const last = runs[runs.length - 1]
    if (last && last.run === entry.run) {
      if (entry.outcome === 'fail') last.outcome = 'fail'
    } else runs.push({ run: entry.run, at: entry.at, outcome: entry.outcome })
  }
  return runs
}

/** STRIP_RUNS checks as a row of marks, oldest first, and one sentence about the streak. */
const STRIP_RUNS = 14

function TrackRecord({ entries, t, now }: { entries: HistoryEntry[]; t: Labels; now: number }) {
  const runs = runsOf(entries)
  let sentence = t.lifeNever
  if (runs.length) {
    const head = runs[0]!.outcome
    const streak = runs.findIndex((r) => r.outcome !== head)
    const n = streak === -1 ? runs.length : streak
    if (head === 'pass') sentence = n === 1 ? t.lifeStreakOne : t.lifeStreak.replace('{n}', String(n))
    else {
      sentence = n === 1 ? t.lifeFailingOne : t.lifeFailing.replace('{n}', String(n))
      const worked = streak === -1 ? null : runs[streak]
      if (worked) sentence += ` ${t.lifeLastWorked.replace('{age}', fmtAge(worked.at, now))}`
    }
  }
  const strip = runs.slice(0, STRIP_RUNS).reverse()
  return (
    <section className="grid gap-1.5" aria-label={t.lifeLabel}>
      <h3 className="text-muted-foreground m-0 text-[11px] font-semibold tracking-wider uppercase">{t.lifeLabel}</h3>
      {strip.length > 0 && (
        <div className="flex flex-wrap gap-0.5" role="img" aria-label={t.lifeStrip.replace('{n}', String(strip.length))}>
          {strip.map((r) => (
            <span
              key={r.run}
              title={`${r.outcome === 'pass' ? t.pass : t.fail} · ${t.ago.replace('{age}', fmtAge(r.at, now))}`}
              className={cn('h-4 w-2.5 rounded-sm', r.outcome === 'pass' ? 'bg-ok' : 'bg-nf')}
            />
          ))}
        </div>
      )}
      <p className="text-muted-foreground m-0 text-sm">{sentence}</p>
    </section>
  )
}

function Disclosure({ open, onToggle, label }: { open: boolean; onToggle: () => void; label: string }) {
  return (
    <button
      type="button"
      aria-expanded={open}
      onClick={onToggle}
      className="text-muted-foreground hover:text-foreground inline-flex cursor-pointer items-center gap-0.5 rounded px-2 py-1 text-xs"
    >
      {label}
      <ChevronRight className={cn('size-3.5 transition-transform', open && 'rotate-90')} aria-hidden="true" />
    </button>
  )
}

/** Results grouped by the run that reported them, newest run first. */
function History({ entries, t, now }: { entries: HistoryEntry[]; t: Labels; now: number }) {
  if (!entries.length) return <p className="text-muted-foreground m-0 text-sm">{t.noHistory}</p>
  const runs: { run: string; first: HistoryEntry; items: HistoryEntry[] }[] = []
  for (const entry of entries) {
    const last = runs[runs.length - 1]
    if (last && last.run === entry.run) last.items.push(entry)
    else runs.push({ run: entry.run, first: entry, items: [entry] })
  }
  return (
    <ol className="m-0 grid list-none gap-3 p-0" aria-label={t.history}>
      {runs.map(({ run, first, items }) => (
        <li key={run} className="min-w-0 text-xs">
          <p className="text-muted-foreground m-0">
            {t.ago.replace('{age}', fmtAge(first.at, now))}
            {first.commit && ` · ${first.commit.slice(0, 7)}`} · {first.actor}
          </p>
          {first.note && <p className="m-0 break-words italic">{first.note}</p>}
          <ul className="m-0 mt-1 grid list-none gap-1 p-0">
            {items.map((entry) => (
              <li key={entry.test} className="flex min-w-0 items-start gap-1.5">
                <StatusIcon status={entry.outcome === 'pass' ? 'verified' : 'failing'} className="size-3.5" />
                <span className="min-w-0 break-words">
                  {describeTest(entry.test).name}
                  {entry.detail && <span className="text-muted-foreground"> — {entry.detail}</span>}
                </span>
              </li>
            ))}
          </ul>
        </li>
      ))}
    </ol>
  )
}
