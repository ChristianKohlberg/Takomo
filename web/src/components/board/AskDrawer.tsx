// Ask a human, from the ticket you are looking at.
//
// The board is where you notice that a decision is needed, so this is where the
// question gets asked — it lands in /inbox like any other. A BLOCKING question
// parks the ticket and releases the agent's lease; an ADVISORY one records a
// routed decision and changes no state. That choice is the first thing the form
// asks, because it is the consequential one.
import { useState } from 'react'
import { Field } from '@/components/Field'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { cn } from '@/lib/utils'
import { splitList } from '@/lib/format'
import type { QuestionKind, QuestionMode } from '@/lib/questions'

export interface AskFields {
  ticket: string
  kind: QuestionKind
  mode: QuestionMode
  title: string
  body?: string
  options?: string[]
  expertise?: string[]
}

export interface AskDrawerProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  ticket: string
  /** The project's question language, when it sets one. */
  languageHint?: string
  onAsk: (fields: AskFields) => Promise<unknown>
  labels: {
    title: string
    subtitle: string
    fTicket: string
    fKind: string
    fMode: string
    fTitle: string
    fBody: string
    fOptions: string
    fOptionsHint: string
    fExpertise: string
    fExpertiseHint: string
    blocking: string
    advisory: string
    blockingHint: string
    advisoryHint: string
    langHint: string
    ask: string
    cancel: string
    needTitle: string
  }
}

const KINDS: QuestionKind[] = ['confirm', 'choose', 'clarify', 'approve']

export function AskDrawer({
  open,
  onOpenChange,
  ticket,
  languageHint,
  onAsk,
  labels,
}: AskDrawerProps) {
  const [kind, setKind] = useState<QuestionKind>('confirm')
  const [mode, setMode] = useState<QuestionMode>('blocking')
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [options, setOptions] = useState('')
  const [expertise, setExpertise] = useState('')
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  async function submit() {
    if (!title.trim()) {
      setErr(labels.needTitle)
      return
    }
    const fields: AskFields = { ticket, kind, mode, title: title.trim() }
    if (body.trim()) fields.body = body.trim()
    if (kind === 'choose' && splitList(options).length) fields.options = splitList(options)
    if (splitList(expertise).length) fields.expertise = splitList(expertise)

    setBusy(true)
    setErr('')
    try {
      await onAsk(fields)
    } catch (e) {
      setErr((e as Error)?.message ?? String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[86vh] max-w-140 overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{labels.title}</DialogTitle>
          <DialogDescription>{labels.subtitle}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <div className="text-muted-foreground font-mono text-[12px]">
            {labels.fTicket}: {ticket}
          </div>

          {/* Mode first: it is the consequential choice, not a detail. */}
          <Field label={labels.fMode} hint={mode === 'blocking' ? labels.blockingHint : labels.advisoryHint}>
            {() => (
              <div className="flex gap-2">
                {(['blocking', 'advisory'] as const).map((m) => (
                  <button
                    key={m}
                    type="button"
                    onClick={() => setMode(m)}
                    className={cn(
                      'border-border cursor-pointer rounded-lg border px-3 py-1.5 text-[13px] font-[650]',
                      mode === m && 'bg-primary text-primary-foreground border-transparent',
                    )}
                  >
                    {m === 'blocking' ? labels.blocking : labels.advisory}
                  </button>
                ))}
              </div>
            )}
          </Field>

          <Field label={labels.fKind}>
            {(id) => (
              <select
                id={id}
                value={kind}
                onChange={(e) => setKind(e.target.value as QuestionKind)}
                className="border-border bg-card text-foreground w-full rounded-lg border px-2.5 py-1.5 text-[13px]"
              >
                {KINDS.map((k) => (
                  <option key={k} value={k}>
                    {k}
                  </option>
                ))}
              </select>
            )}
          </Field>

          <Field label={labels.fTitle} hint={languageHint ? labels.langHint.replace('{lang}', languageHint) : undefined}>
            {(id) => <Input id={id} autoFocus value={title} onChange={(e) => setTitle(e.target.value)} />}
          </Field>

          <Field label={labels.fBody}>
            {(id) => (
              <Textarea id={id} className="min-h-20" value={body} onChange={(e) => setBody(e.target.value)} />
            )}
          </Field>

          {/* Options only exist for `choose` — offering them elsewhere would
              suggest an answer shape the question cannot take. */}
          {kind === 'choose' && (
            <Field label={labels.fOptions} hint={labels.fOptionsHint}>
              {(id) => (
                <Input id={id} value={options} onChange={(e) => setOptions(e.target.value)} />
              )}
            </Field>
          )}

          <Field label={labels.fExpertise} hint={labels.fExpertiseHint}>
            {(id) => (
              <Input id={id} value={expertise} onChange={(e) => setExpertise(e.target.value)} />
            )}
          </Field>

          <div className="text-destructive min-h-4 text-[12.5px]">{err}</div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {labels.cancel}
          </Button>
          <Button onClick={submit} disabled={busy}>
            {labels.ask}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
