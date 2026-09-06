import { useState } from 'react'
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { api } from '@/lib/api'
import type { Ticket } from '@/lib/board'
import type { Locale } from '@/lib/i18n'

const COPY = {
  en: { title: 'New epic', hint: 'Describe the outcome this epic brings together.', name: 'Title', body: 'Description (optional)', create: 'Create epic', cancel: 'Cancel', error: 'Could not create epic.' },
  de: { title: 'Neues Epic', hint: 'Beschreibe das Ziel, das dieses Epic zusammenführt.', name: 'Titel', body: 'Beschreibung (optional)', create: 'Epic erstellen', cancel: 'Abbrechen', error: 'Epic konnte nicht erstellt werden.' },
}

export function CreateEpicDialog({ open, onOpenChange, token, project, lang, onCreated }: {
  open: boolean
  onOpenChange: (open: boolean) => void
  token: string
  project: string
  lang: Locale
  onCreated: (ticket: Ticket) => void
}) {
  const t = COPY[lang]
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  return <Dialog open={open} onOpenChange={(next) => { if (!saving) { setError(''); onOpenChange(next) } }}>
    <DialogContent>
      <DialogHeader><DialogTitle>{t.title}</DialogTitle><DialogDescription>{t.hint}</DialogDescription></DialogHeader>
      <form className="flex min-w-0 flex-col gap-4" onSubmit={async (event) => {
        event.preventDefault()
        if (!title.trim() || saving) return
        setSaving(true)
        setError('')
        try {
          // Omit state: the server applies this project's workflow initial state.
          const ticket = await api<Ticket>(token, '/tickets', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ project, type: 'epic', title: title.trim(), body: body.trim() || undefined }) })
          setTitle('')
          setBody('')
          onCreated(ticket)
        } catch (e) { setError(e instanceof Error ? e.message : t.error) }
        finally { setSaving(false) }
      }}>
        <label className="flex flex-col gap-1.5 text-sm">{t.name}<Input autoFocus required value={title} onChange={(e) => setTitle(e.target.value)} disabled={saving} /></label>
        <label className="flex flex-col gap-1.5 text-sm">{t.body}<textarea className="border-input bg-background min-h-28 rounded-md border p-2" value={body} onChange={(e) => setBody(e.target.value)} disabled={saving} /></label>
        {error && <p role="alert" className="text-destructive text-sm">{error}</p>}
        <div className="flex justify-end gap-2"><Button type="button" variant="outline" disabled={saving} onClick={() => onOpenChange(false)}>{t.cancel}</Button><Button type="submit" disabled={saving || !title.trim()}>{t.create}</Button></div>
      </form>
    </DialogContent>
  </Dialog>
}
