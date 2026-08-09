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
import { DAYS, UNITS, guessTz, type Cadence, type CreateFields, type Unit } from '@/lib/schedules'

export interface CreateScheduleDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  project: string
  onCreate: (fields: CreateFields) => Promise<unknown>
  labels: {
    title: string
    subtitle: string
    fName: string
    fNamePh: string
    fEvery: string
    fInterval: string
    fDays: string
    fDayOfMonth: string
    fAt: string
    fTz: string
    fTitle: string
    fTitlePh: string
    fTitleHint: string
    fBody: string
    fLabels: string
    fLabelsPh: string
    fRationale: string
    fRationalePh: string
    unitDay: string
    unitWeek: string
    unitMonth: string
    create: string
    cancel: string
    pickDay: string
  }
}

export function CreateScheduleDialog({
  open,
  onOpenChange,
  project,
  onCreate,
  labels,
}: CreateScheduleDialogProps) {
  const [name, setName] = useState('')
  const [unit, setUnit] = useState<Unit>('week')
  const [interval, setInterval] = useState(1)
  const [days, setDays] = useState<string[]>(['mon'])
  const [dayOfMonth, setDayOfMonth] = useState(1)
  const [at, setAt] = useState('09:00')
  const [tz, setTz] = useState(guessTz())
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [labelsCsv, setLabelsCsv] = useState('')
  const [rationale, setRationale] = useState('')
  const [err, setErr] = useState('')
  const [busy, setBusy] = useState(false)

  const unitLabel = (u: Unit) =>
    u === 'day' ? labels.unitDay : u === 'week' ? labels.unitWeek : labels.unitMonth

  // See the note in AskDrawer: this dialog stays mounted when closed, so
  // without an explicit reset the next "New schedule" opens holding the last
  // one's name, cadence and ticket template.
  function reset() {
    setName('')
    setUnit('week')
    setInterval(1)
    setDays(['mon'])
    setDayOfMonth(1)
    setAt('09:00')
    setTz(guessTz())
    setTitle('')
    setBody('')
    setLabelsCsv('')
    setRationale('')
    setErr('')
    setBusy(false)
  }

  async function submit() {
    const cadence: Cadence = { every: unit, at, tz: tz.trim() || 'UTC' }
    if (interval > 1) cadence.interval = interval
    if (unit === 'week') {
      if (!days.length) {
        setErr(labels.pickDay)
        return
      }
      cadence.on = [...days]
    }
    if (unit === 'month') cadence.day = dayOfMonth

    const fields: CreateFields = {
      project,
      name: name.trim(),
      cadence,
      template: { title: title.trim() },
    }
    if (body.trim()) fields.template.body = body.trim()
    if (splitList(labelsCsv).length) fields.template.labels = splitList(labelsCsv)
    if (rationale.trim()) fields.rationale = rationale.trim()

    setBusy(true)
    setErr('')
    try {
      await onCreate(fields)
    } catch (e) {
      setErr((e as Error)?.message ?? String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(o) => {
        if (!o) reset()
        onOpenChange(o)
      }}
    >
      <DialogContent className="max-h-[86vh] max-w-[calc(100%-2rem)] sm:max-w-140 overflow-y-auto">
        <DialogHeader>
          <DialogTitle>{labels.title}</DialogTitle>
          <DialogDescription>{labels.subtitle}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-3">
          <Field label={labels.fName}>
            {(id) => (
              <Input id={id} autoFocus placeholder={labels.fNamePh} value={name} onChange={(e) => setName(e.target.value)} />
            )}
          </Field>

          <div className="flex flex-wrap gap-2.5 [&>*]:flex-[1_1_170px]">
            <Field label={labels.fEvery}>
              {(id) => (
                <select
                  id={id}
                  value={unit}
                  onChange={(e) => setUnit(e.target.value as Unit)}
                  className="border-border bg-card text-foreground w-full rounded-lg border px-2.5 py-1.5 text-[13px]"
                >
                  {UNITS.map((u) => (
                    <option key={u} value={u}>
                      {unitLabel(u)}
                    </option>
                  ))}
                </select>
              )}
            </Field>
            <Field label={labels.fInterval}>
              {(id) => (
                <Input
                  id={id}
                  type="number"
                  min={1}
                  max={52}
                  value={interval}
                  onChange={(e) => setInterval(Math.max(1, Number(e.target.value) || 1))}
                />
              )}
            </Field>
            <Field label={labels.fAt}>
              {(id) => <Input id={id} type="time" step={60} value={at} onChange={(e) => setAt(e.target.value)} />}
            </Field>
          </div>

          {/* The two cadence-specific fields are shown only for the unit that uses
              them — a day-of-month picker on a weekly cadence is a lie. */}
          {unit === 'week' && (
            <Field label={labels.fDays}>
              {() => (
                <div className="flex flex-wrap gap-1">
                  {DAYS.map((d) => (
                    <button
                      key={d}
                      type="button"
                      onClick={() =>
                        setDays((cur) => (cur.includes(d) ? cur.filter((x) => x !== d) : [...cur, d]))
                      }
                      className={cn(
                        'border-border text-muted-foreground cursor-pointer rounded-lg border px-2.25 py-1.25 text-[12px] font-[650]',
                        days.includes(d) && 'bg-primary text-primary-foreground border-transparent',
                      )}
                    >
                      {d}
                    </button>
                  ))}
                </div>
              )}
            </Field>
          )}
          {unit === 'month' && (
            <Field label={labels.fDayOfMonth}>
              {(id) => (
                <Input
                  id={id}
                  type="number"
                  min={1}
                  max={31}
                  value={dayOfMonth}
                  onChange={(e) => setDayOfMonth(Math.min(31, Math.max(1, Number(e.target.value) || 1)))}
                />
              )}
            </Field>
          )}

          <Field label={labels.fTz}>
            {(id) => <Input id={id} placeholder="Europe/Berlin" value={tz} onChange={(e) => setTz(e.target.value)} />}
          </Field>

          <Field label={labels.fTitle} hint={labels.fTitleHint}>
            {(id) => (
              <Input id={id} placeholder={labels.fTitlePh} value={title} onChange={(e) => setTitle(e.target.value)} />
            )}
          </Field>
          <Field label={labels.fBody}>
            {(id) => <Textarea id={id} className="min-h-16" value={body} onChange={(e) => setBody(e.target.value)} />}
          </Field>
          <Field label={labels.fLabels}>
            {(id) => (
              <Input id={id} placeholder={labels.fLabelsPh} value={labelsCsv} onChange={(e) => setLabelsCsv(e.target.value)} />
            )}
          </Field>
          <Field label={labels.fRationale}>
            {(id) => (
              <Textarea
                id={id}
                className="min-h-16"
                placeholder={labels.fRationalePh}
                value={rationale}
                onChange={(e) => setRationale(e.target.value)}
              />
            )}
          </Field>

          <div className="text-destructive min-h-4 text-[12.5px]">{err}</div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {labels.cancel}
          </Button>
          <Button onClick={submit} disabled={busy}>
            {labels.create}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
