import { useEffect, useRef, useState } from 'react'
import { useCollaboration } from '@/hooks/useCollaboration'
import { api } from '@/lib/api'
import { runRequest, type TestDefinition, type TestRun } from '@/lib/test-runs'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Dialog, DialogContent, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { useSpecification } from '../specification/context'
import type { Environment } from '@/lib/verification'

/** Reopen the durable replica, including edits left offline in a closed editor.
 * Creating a run requires its ordered server acknowledgment, not merely a socket. */
export function RunComposer({ check, environments, close, created }: { check: string; environments: Environment[]; close: () => void; created: (run: TestRun) => void }) {
  const { token, project, saveState: planSave, map, onError, lang } = useSpecification()
  const de = lang === 'de'
  const { saveState } = useCollaboration(token, `/checks/${encodeURIComponent(check)}/session`, onError)
  const [definition, setDefinition] = useState<TestDefinition | null>(null)
  const [code, setCode] = useState('')
  const [environment, setEnvironment] = useState('')
  const [busy, setBusy] = useState(false)
  const attempt = useRef<{ fingerprint: string; key: string } | null>(null)
  const synced = saveState === 'saved' && (!map || planSave === 'saved')
  useEffect(() => {
    if (!synced) return
    let active = true
    void api<TestDefinition>(token, `/checks/${encodeURIComponent(check)}/definition`).then(d => {
      if (active) { setDefinition(d); if (d.definition.environments.length === 1) setEnvironment(d.definition.environments[0]!) }
    }).catch(onError)
    return () => { active = false }
  }, [check, token, synced, onError])
  const available = environments.filter(e => !definition?.definition.environments.length || definition.definition.environments.includes(e.id))
  return <Dialog open onOpenChange={open => { if (!open && !busy) close() }}>
    <DialogContent className="max-h-[90dvh] overflow-y-auto">
      <DialogHeader><DialogTitle>{de ? 'Testlauf erstellen' : 'Create test run'}</DialogTitle></DialogHeader>
      <p className="text-sm text-muted-foreground">{de ? 'Dieser Lauf hält die Definition und den Spezifikationsstand fest. Ein Agent oder eine Person führt die Fälle aus.' : 'This run captures the definition and specification revisions. An agent or person executes its cases.'}</p>
      <p className="font-semibold">{definition?.definition.title}</p>
      <label className="grid gap-1 text-sm">{de ? 'Codeversion (Commit oder unveränderlicher Build)' : 'Code version (commit or immutable build)'}<Input disabled={busy} value={code} onChange={e => setCode(e.target.value)} maxLength={300} /></label>
      <label className="grid gap-1 text-sm">{de ? 'Umgebung' : 'Environment'}<select className="h-9 min-w-0 rounded-md border bg-background px-2" disabled={busy} value={environment} onChange={e => setEnvironment(e.target.value)}><option value="">{de ? 'Ohne Umgebung' : 'No environment'}</option>{available.map(e => <option key={e.id} value={e.id}>{e.name}</option>)}</select></label>
      {!synced && <p role="status" className="text-sm text-muted-foreground">{de ? 'Warte auf gespeicherte Änderungen …' : 'Waiting for edits to be saved …'}</p>}
      <Button disabled={busy || !synced || !definition || !code.trim() || (!!definition.definition.environments.length && !environment)} onClick={() => {
        if (!definition) return
        setBusy(true)
        const request = {
          definitions: [{ check, definition_revision: definition.definition_revision, specification_revision: definition.specification_revision }],
          environment: environment || null, code_ref: code.trim(),
        }
        const fingerprint = JSON.stringify(request)
        if (attempt.current?.fingerprint !== fingerprint) attempt.current = { fingerprint, key: crypto.randomUUID() }
        void runRequest<TestRun>(token, `/projects/${encodeURIComponent(project)}/test-runs`, { ...request, idempotency_key: attempt.current.key }).then(created).catch(error => {
          onError(error)
          // A revision conflict needs an explicit fresh selection, preserving code/environment.
          if ((error as { code?: string }).code === 'conflict.definition_changed') void api<TestDefinition>(token, `/checks/${encodeURIComponent(check)}/definition`).then(setDefinition).catch(onError)
        }).finally(() => setBusy(false))
      }}>{busy ? (de ? 'Erstellen …' : 'Creating …') : (de ? 'Lauf erstellen' : 'Create run')}</Button>
    </DialogContent>
  </Dialog>
}
