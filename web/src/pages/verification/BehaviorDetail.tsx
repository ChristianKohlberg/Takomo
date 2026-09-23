import { useCallback, useEffect, useRef, useState } from 'react'
import { Markdown } from '@/components/Markdown'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import {
  deleteBehavior,
  getBehavior,
  patchBehavior,
  resultStamp,
  type BehaviorDetail as Detail,
} from '@/lib/behaviors'
import type { PlanNode } from '@/lib/plan-sections'
import { SectionSelect } from './SectionSelect'
import { OutcomeMark, StatusBadge } from './status'
import type { STR } from './strings'

type Labels = (typeof STR)['en']

/**
 * One behavior: its text, section, linked tests with their latest results, and
 * the history of those results. Plain fields saved with PATCH — a behavior is a
 * short statement, not a document edited together.
 */
export function BehaviorDetail({
  token,
  id,
  nodes,
  canWrite,
  t,
  version,
  onBack,
  onChanged,
  onDeleted,
  onError,
}: {
  token: string
  id: string
  nodes: PlanNode[]
  canWrite: boolean
  t: Labels
  /** Bumped by the parent when a live update may have changed this behavior. */
  version: number
  onBack: () => void
  onChanged: () => void
  onDeleted: () => void
  onError: (error: unknown) => void
}) {
  const [detail, setDetail] = useState<Detail | null>(null)
  const [missing, setMissing] = useState('')
  const [title, setTitle] = useState('')
  const [statement, setStatement] = useState('')
  const [newTest, setNewTest] = useState('')
  const [busy, setBusy] = useState(false)
  const [message, setMessage] = useState('')
  const epoch = useRef(0)
  // The text last read from the server. A reload replaces a field only when the
  // reader has not changed it since, so a live update never discards typing.
  const loaded = useRef({ title: '', statement: '' })

  const load = useCallback(async () => {
    const attempt = ++epoch.current
    try {
      const next = await getBehavior(token, id)
      if (attempt !== epoch.current) return
      const previous = loaded.current
      loaded.current = { title: next.title, statement: next.statement }
      setDetail(next)
      setTitle((current) => (current === previous.title ? next.title : current))
      setStatement((current) => (current === previous.statement ? next.statement : current))
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
      setMessage(t.saved)
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
      <div className="grid gap-3">
        <Button variant="ghost" size="sm" className="justify-self-start" onClick={onBack}>
          ← {t.back}
        </Button>
        <p role="alert" className="text-sm break-words">
          {missing}
        </p>
      </div>
    )
  if (!detail) return <p role="status">{t.loading}</p>

  const dirty = title.trim() !== detail.title || statement.trim() !== detail.statement
  const addTest = () => {
    const key = newTest.trim()
    if (!key || detail.tests.includes(key)) return
    // Clear the field only once the key is saved, so a failed save keeps it.
    void save({ add_tests: [key] }).then((saved) => {
      if (saved) setNewTest('')
    })
  }

  return (
    <article className="grid min-w-0 gap-4" aria-label={detail.title}>
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="ghost" size="sm" onClick={onBack}>
          ← {t.back}
        </Button>
        <span className="grow" />
        <StatusBadge status={detail.status} t={t} />
      </div>

      {canWrite ? (
        <form
          className="grid gap-3"
          onSubmit={(event) => {
            event.preventDefault()
            if (!title.trim()) {
              setMessage(t.titleRequired)
              return
            }
            void save({ title: title.trim(), statement: statement.trim() })
          }}
        >
          <div className="grid gap-1.5">
            <Label htmlFor="behavior-edit-title">{t.fTitle}</Label>
            <Input id="behavior-edit-title" value={title} onChange={(event) => setTitle(event.target.value)} />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="behavior-edit-statement">{t.fStatement}</Label>
            <Textarea
              id="behavior-edit-statement"
              rows={5}
              value={statement}
              placeholder={t.fStatementPh}
              onChange={(event) => setStatement(event.target.value)}
            />
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button type="submit" size="sm" disabled={busy || !dirty}>
              {t.save}
            </Button>
            {message && (
              <span role="status" className="text-muted-foreground text-xs">
                {message}
              </span>
            )}
          </div>
        </form>
      ) : (
        <div className="grid gap-2">
          <h2 className="text-lg font-semibold break-words">{detail.title}</h2>
          {detail.statement && <Markdown text={detail.statement} className="text-sm" />}
        </div>
      )}

      <div className="grid gap-1.5">
        <Label htmlFor="behavior-edit-section">{t.fSection}</Label>
        {canWrite ? (
          <SectionSelect
            id="behavior-edit-section"
            nodes={nodes}
            value={detail.section}
            noneLabel={t.noSection}
            onChange={(section) => void save({ section })}
          />
        ) : (
          <p id="behavior-edit-section" className="m-0 text-sm">
            {nodes.find((node) => node.id === detail.section)?.title ?? detail.section ?? t.noSection}
          </p>
        )}
      </div>

      <section className="grid gap-2">
        <h3 className="text-sm font-semibold">{t.linkedTests}</h3>
        {detail.test_results.length === 0 && <p className="text-muted-foreground m-0 text-sm">{t.noTests}</p>}
        <ul className="m-0 grid list-none gap-2 p-0">
          {detail.test_results.map((item) => (
            <li key={item.test} className="bg-muted min-w-0 rounded-md px-3 py-2">
              <div className="flex min-w-0 items-start gap-2">
                <code className="min-w-0 flex-1 text-xs break-all">{item.test}</code>
                {canWrite && (
                  <button
                    type="button"
                    aria-label={t.removeTest.replace('{test}', item.test)}
                    className="text-muted-foreground hover:text-foreground cursor-pointer text-sm leading-none"
                    disabled={busy}
                    onClick={() => void save({ remove_tests: [item.test] })}
                  >
                    ×
                  </button>
                )}
              </div>
              <p className="text-muted-foreground m-0 mt-1 text-xs">
                {item.latest ? (
                  <>
                    <OutcomeMark outcome={item.latest.outcome} label={item.latest.outcome === 'pass' ? t.pass : t.fail} />
                    {' · '}
                    {resultStamp(item.latest)}
                  </>
                ) : (
                  t.notReported
                )}
              </p>
              {item.latest?.detail && (
                <p className="m-0 mt-1 text-xs break-words whitespace-pre-wrap">{item.latest.detail}</p>
              )}
            </li>
          ))}
        </ul>
        {canWrite && (
          <form
            className="flex min-w-0 gap-2"
            onSubmit={(event) => {
              event.preventDefault()
              addTest()
            }}
          >
            <Input
              aria-label={t.addTestPh}
              placeholder={t.addTestPh}
              value={newTest}
              className="min-w-0 flex-1 font-mono text-xs"
              onChange={(event) => setNewTest(event.target.value)}
            />
            <Button type="submit" size="sm" variant="outline" disabled={busy || !newTest.trim()}>
              {t.addTest}
            </Button>
          </form>
        )}
      </section>

      <section className="grid gap-2">
        <h3 className="text-sm font-semibold">{t.history}</h3>
        {detail.history.length === 0 ? (
          <p className="text-muted-foreground m-0 text-sm">{t.noHistory}</p>
        ) : (
          <ol className="m-0 grid list-none gap-1.5 p-0">
            {detail.history.map((entry, index) => (
              <li key={`${entry.run}-${entry.test}-${index}`} className="min-w-0 border-b pb-1.5 text-xs last:border-b-0">
                <div className="flex flex-wrap items-baseline gap-x-2">
                  <OutcomeMark outcome={entry.outcome} label={entry.outcome === 'pass' ? t.pass : t.fail} />
                  <code className="min-w-0 break-all">{entry.test}</code>
                  <span className="text-muted-foreground">
                    {resultStamp(entry)} · {entry.actor}
                  </span>
                </div>
                {entry.detail && <p className="m-0 mt-0.5 break-words whitespace-pre-wrap">{entry.detail}</p>}
                {entry.note && <p className="text-muted-foreground m-0 mt-0.5 break-words">{entry.note}</p>}
              </li>
            ))}
          </ol>
        )}
      </section>

      {canWrite && (
        <div>
          <Button
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
      )}
    </article>
  )
}
