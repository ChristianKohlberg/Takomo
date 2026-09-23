import { useEffect, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Textarea } from '@/components/ui/textarea'
import { parseTestKeys, type BehaviorFields } from '@/lib/behaviors'
import type { PlanNode } from '@/lib/plan-sections'
import { SectionSelect } from './SectionSelect'

export interface BehaviorDialogLabels {
  newBehavior: string
  fTitle: string
  fTitlePh: string
  fStatement: string
  fStatementPh: string
  fStatementHint: string
  fSection: string
  noSection: string
  fTests: string
  fTestsPh: string
  fTestsHint: string
  create: string
  cancel: string
  titleRequired: string
}

export function BehaviorDialog({
  open,
  onOpenChange,
  nodes,
  defaultSection,
  labels: t,
  onSubmit,
}: {
  open: boolean
  onOpenChange: (open: boolean) => void
  nodes: PlanNode[]
  defaultSection: string | null
  labels: BehaviorDialogLabels
  onSubmit: (fields: BehaviorFields) => Promise<void>
}) {
  const [title, setTitle] = useState('')
  const [statement, setStatement] = useState('')
  const [section, setSection] = useState<string | null>(defaultSection)
  const [tests, setTests] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    if (!open) return
    setTitle('')
    setStatement('')
    setSection(defaultSection)
    setTests('')
    setError('')
  }, [open, defaultSection])

  const submit = async () => {
    if (!title.trim()) {
      setError(t.titleRequired)
      return
    }
    setBusy(true)
    try {
      await onSubmit({
        title: title.trim(),
        statement: statement.trim(),
        ...(section ? { section } : {}),
        tests: parseTestKeys(tests),
      })
      onOpenChange(false)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-[calc(100vw-2rem)] sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t.newBehavior}</DialogTitle>
        </DialogHeader>
        <form
          className="grid min-w-0 gap-3"
          onSubmit={(event) => {
            event.preventDefault()
            void submit()
          }}
        >
          <div className="grid gap-1.5">
            <Label htmlFor="behavior-title">{t.fTitle}</Label>
            <Input
              id="behavior-title"
              value={title}
              placeholder={t.fTitlePh}
              onChange={(event) => setTitle(event.target.value)}
              autoFocus
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="behavior-statement">{t.fStatement}</Label>
            <Textarea
              id="behavior-statement"
              value={statement}
              rows={4}
              placeholder={t.fStatementPh}
              onChange={(event) => setStatement(event.target.value)}
            />
            <p className="text-muted-foreground m-0 text-xs">{t.fStatementHint}</p>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="behavior-section">{t.fSection}</Label>
            <SectionSelect
              id="behavior-section"
              nodes={nodes}
              value={section}
              noneLabel={t.noSection}
              onChange={setSection}
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="behavior-tests">{t.fTests}</Label>
            <Textarea
              id="behavior-tests"
              value={tests}
              rows={3}
              className="font-mono text-xs"
              placeholder={t.fTestsPh}
              onChange={(event) => setTests(event.target.value)}
            />
            <p className="text-muted-foreground m-0 text-xs">{t.fTestsHint}</p>
          </div>
          {error && (
            <p role="alert" className="text-nf m-0 text-sm break-words">
              {error}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" disabled={busy} onClick={() => onOpenChange(false)}>
              {t.cancel}
            </Button>
            <Button type="submit" disabled={busy}>
              {t.create}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  )
}
