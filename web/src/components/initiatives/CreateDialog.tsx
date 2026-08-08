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
import { splitList } from '@/lib/format'
import type { CreateFields } from '@/lib/initiatives'

export interface CreateDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The project the initiative is filed under; the caller guarantees it is set. */
  project: string
  onCreate: (fields: CreateFields) => void
  onInvalid: (message: string) => void
  labels: {
    title: string
    subtitle: string
    fTitle: string
    fTitlePh: string
    fSummary: string
    fSummaryPh: string
    fLabels: string
    fLabelsPh: string
    fTags: string
    fTagsPh: string
    create: string
    cancel: string
    needTitle: string
  }
}

export function CreateDialog({
  open,
  onOpenChange,
  project,
  onCreate,
  onInvalid,
  labels,
}: CreateDialogProps) {
  const [title, setTitle] = useState('')
  const [summary, setSummary] = useState('')
  const [labelsCsv, setLabelsCsv] = useState('')
  const [tagsCsv, setTagsCsv] = useState('')

  function reset() {
    setTitle('')
    setSummary('')
    setLabelsCsv('')
    setTagsCsv('')
  }

  function submit() {
    if (!title.trim()) {
      onInvalid(labels.needTitle)
      return
    }
    const fields: CreateFields = { project, title: title.trim() }
    if (summary.trim()) fields.summary = summary.trim()
    if (splitList(labelsCsv).length) fields.labels = splitList(labelsCsv)
    if (splitList(tagsCsv).length) fields.tags = splitList(tagsCsv)
    onCreate(fields)
    reset()
  }

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) reset()
        onOpenChange(next)
      }}
    >
      <DialogContent className="max-w-140">
        <DialogHeader>
          <DialogTitle>{labels.title}</DialogTitle>
          <DialogDescription>{labels.subtitle}</DialogDescription>
        </DialogHeader>

        <div className="flex flex-col gap-2.5">
          <Field label={labels.fTitle}>
            {(id) => (
              <Input
                id={id}
                autoFocus
                placeholder={labels.fTitlePh}
                value={title}
                onChange={(e) => setTitle(e.target.value)}
              />
            )}
          </Field>
          <Field label={labels.fSummary}>
            {(id) => (
              <Textarea
                id={id}
                className="min-h-20"
                placeholder={labels.fSummaryPh}
                value={summary}
                onChange={(e) => setSummary(e.target.value)}
              />
            )}
          </Field>
          <div className="flex flex-wrap gap-2.5 [&>*]:flex-[1_1_170px]">
            <Field label={labels.fLabels}>
              {(id) => (
                <Input
                  id={id}
                  placeholder={labels.fLabelsPh}
                  value={labelsCsv}
                  onChange={(e) => setLabelsCsv(e.target.value)}
                />
              )}
            </Field>
            <Field label={labels.fTags} hint={labels.fTagsPh}>
              {(id) => (
                <Input
                  id={id}
                  placeholder={labels.fTagsPh}
                  value={tagsCsv}
                  onChange={(e) => setTagsCsv(e.target.value)}
                />
              )}
            </Field>
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {labels.cancel}
          </Button>
          <Button onClick={submit}>{labels.create}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
