import { useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Field } from '@/components/Field'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from '@/components/ui/dialog'
import { createProject } from '@/lib/admin'
import { githubWrite, setRepository, startExtraction, type RepositorySelection } from '@/lib/github'
import { isValidProjectId } from './NewProjectDialog'
import { RepositoryFields } from './RepositoryFields'
export function NewProjectWizard({ open, onOpenChange, token, locale, initialName = '', canConnect, onCreated }: { open: boolean; onOpenChange: (open: boolean) => void; token: string; locale: string; initialName?: string; canConnect: boolean; onCreated: (project: string) => void }) {
  const de = locale === 'de'
  const [step, setStep] = useState(0)
  const [name, setName] = useState(initialName)
  const [id, setId] = useState(initialName.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, ''))
  const [connect, setConnect] = useState(false)
  const [repository, setRepositorySelection] = useState<RepositorySelection | null>(null)
  const [launch, setLaunch] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const created = useRef(false)
  const map = useRef('')
  const request = useRef(crypto.randomUUID())
  async function submit() {
    setBusy(true); setError('')
    try {
      if (!created.current) { await createProject(token, { id, name: name.trim() || id }); created.current = true }
      if (connect && repository) await setRepository(token, id, repository)
      if (connect && launch && repository) {
        if (!map.current) { const result = await githubWrite<{ mindmap: { id: string } }>(token, '/mindmaps', { project: id, title: name.trim() || id }); map.current = result.mindmap.id }
        await startExtraction(token, id, map.current, request.current)
      }
      onOpenChange(false); onCreated(id)
    } catch (e) { setError(`${created.current ? (de ? 'Projekt erstellt. Der nächste Schritt ist fehlgeschlagen: ' : 'Project created. The next step failed: ') : ''}${(e as Error).message}`) } finally { setBusy(false) }
  }
  const titles = de ? ['Projekt erstellen', 'Code verbinden', 'Entwurf starten?'] : ['Create a project', 'Connect your code', 'Start a draft?']
  return <Dialog open={open} onOpenChange={o => { if (!busy) onOpenChange(o) }}><DialogContent className="max-h-[90dvh] max-w-[calc(100%-2rem)] overflow-y-auto sm:max-w-132">
    <DialogHeader><DialogTitle>{titles[step]}</DialogTitle><DialogDescription>{de ? `Schritt ${step + 1} von 3` : `Step ${step + 1} of 3`}</DialogDescription></DialogHeader>
    {step === 0 && <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">{de ? 'Beginne mit einer leeren Spezifikation oder lasse aus einem ausgewählten Codebereich einen ersten Entwurf erstellen.' : 'Start with an empty specification, or generate a first draft from a selected part of your codebase.'}</p>
      <Field label={de ? 'Projektname' : 'Project name'}>{fid => <Input id={fid} autoFocus disabled={created.current} value={name} onChange={e => setName(e.target.value)} />}</Field>
      <Field label={de ? 'Projekt-ID' : 'Project ID'} hint={de ? 'Kleinbuchstaben, Zahlen und Bindestriche. Die ID wird in URLs verwendet und bleibt bestehen.' : 'Lowercase letters, numbers and dashes. Used in URLs and cannot be renamed.'}>{fid => <Input id={fid} disabled={created.current} value={id} aria-invalid={!!id && !isValidProjectId(id)} onChange={e => setId(e.target.value.toLowerCase())} />}</Field>
    </div>}
    {step === 1 && <div className="flex flex-col gap-4">
      <p className="text-muted-foreground text-sm">{de ? 'GitHub ist optional. Du kannst das Repository später in den Projekteinstellungen verbinden oder ändern.' : 'GitHub is optional. Connect or change the repository later in project settings.'}</p>
      <label className="flex items-center gap-2"><input type="checkbox" checked={connect} disabled={!canConnect} onChange={e => { setConnect(e.target.checked); setLaunch(false) }} />{de ? 'Ein GitHub-Repository verbinden' : 'Connect a GitHub repository'}</label>
      {!canConnect && <p className="text-muted-foreground text-sm">{de ? 'Ein Administrator mit Zugriff auf alle Projekte kann GitHub verbinden.' : 'An administrator with access to all projects can connect GitHub.'}</p>}
      {connect && <RepositoryFields token={token} locale={locale} value={repository} onChange={value => { setRepositorySelection(value); request.current = crypto.randomUUID() }} />}
    </div>}
    {step === 2 && <div className="flex flex-col gap-4">
      <p className="text-sm">{de ? 'Die Extraktion liest den ausgewählten Code und erstellt Abschnitte mit Quellenverweisen. Du prüfst, bearbeitest und bestätigst sie im vorhandenen Dokument oder in der Mindmap.' : 'Extraction reads the selected code and creates sections with source references. Review, edit and confirm them in the existing document or mindmap.'}</p>
      <p className="text-muted-foreground text-sm">{de ? 'Ein Entwurf beschreibt den erkannten Ist-Zustand. Er kann Lücken und Fehler enthalten und führt keine Tests aus.' : 'The draft describes observed behavior. It can contain gaps and mistakes, and does not run tests.'}</p>
      {connect && repository ? <>
        <div className="bg-muted rounded-lg p-3 text-sm break-words"><strong>{repository.full_name}</strong><p>{repository.scope.include.join(', ')}</p><p>{de ? 'Maximal 20 Dateien, 100 KB Quelltext, 3 Abschnitte. Ein Modelllauf; Kosten sind möglich.' : 'Up to 20 files, 100 KB of source and 3 sections. One model run; usage may incur costs.'}</p></div>
        <label className="flex items-center gap-2"><input type="checkbox" checked={launch} onChange={e => setLaunch(e.target.checked)} />{de ? 'Extraktion nach dem Erstellen starten' : 'Start extraction after creating the project'}</label>
        <p className="text-muted-foreground text-sm">{de ? 'Ein verfügbarer Agent führt die Extraktion im Hintergrund aus. Fortschritt und Fehler findest du in den Projekteinstellungen.' : 'An available agent runs the extraction in the background. Find status and errors in project settings.'}</p>
      </> : <p>{de ? 'Dein Projekt startet mit einer leeren Spezifikation.' : 'Your project will start with an empty specification.'}</p>}
    </div>}
    {error && <p role="alert" className="text-destructive text-sm">{error}</p>}
    <DialogFooter><Button variant="ghost" disabled={busy} onClick={() => onOpenChange(false)}>{de ? 'Schließen' : 'Close'}</Button>{step > 0 && <Button variant="secondary" disabled={busy} onClick={() => setStep(s => s - 1)}>{de ? 'Zurück' : 'Back'}</Button>}{step < 2 ? <Button disabled={!isValidProjectId(id) || (step === 1 && connect && (!repository || !repository.scope.include[0]?.trim()))} onClick={() => setStep(s => s + 1)}>{de ? 'Weiter' : 'Continue'}</Button> : <Button disabled={busy} onClick={() => void submit()}>{busy ? (de ? 'Wird gespeichert…' : 'Saving…') : launch && connect ? (de ? 'Erstellen und Extraktion starten' : 'Create and start extraction') : (de ? 'Projekt erstellen' : 'Create project')}</Button>}</DialogFooter>
  </DialogContent></Dialog>
}
