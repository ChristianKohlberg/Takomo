import { useEffect, useState } from 'react'
import * as Y from 'yjs'
import { SharedText } from '@/components/SharedText'
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription } from '@/components/ui/dialog'
import { useCollaboration } from '@/hooks/useCollaboration'

export function CheckEditor({ token, id, onClose, onError, labels }: {
  token: string; id: string; onClose: () => void; onError: (error: unknown) => void
  labels: { fTitle: string; fBody: string; fPrecondition: string; liveEdit: string; liveHint: string; connecting: string; live: string; reconnecting: string }
}) {
  const { client, ready, state, peers } = useCollaboration(token, `/checks/${encodeURIComponent(id)}/session`, onError)
  const [definition, setDefinition] = useState<Y.Map<unknown> | null>(null)
  useEffect(() => {
    if (!client) { setDefinition(null); return }
    const read = () => {
      const value = client.doc.getMap('nodes').get('definition')
      setDefinition(value instanceof Y.Map ? value : null)
    }
    read()
    client.doc.on('update', read)
    return () => { client.doc.off('update', read) }
  }, [client])
  return <Dialog open onOpenChange={open => { if (!open) onClose() }}>
    <DialogContent className="max-h-[85dvh] overflow-y-auto">
      <DialogHeader><DialogTitle>{labels.liveEdit}</DialogTitle><DialogDescription>{labels.liveHint}</DialogDescription></DialogHeader>
      <div role="status" className="text-xs text-muted-foreground">{labels[state]}{peers.length > 0 && ` · ${peers.map(p => p.name).join(', ')}`}</div>
      {(['title', 'precondition', 'body'] as const).map(field => {
        const text = definition?.get(field)
        const label = field === 'title' ? labels.fTitle : field === 'body' ? labels.fBody : labels.fPrecondition
        return text instanceof Y.Text ? <label key={field} className="grid min-w-0 gap-2 text-sm font-medium">{label}
          <SharedText text={text} label={label} readOnly={!ready || !client?.session.can_write}
            maxLength={field === 'title' ? 300 : 65536} className={field === 'title' ? 'border' : 'min-h-28 border'} />
        </label> : null
      })}
    </DialogContent>
  </Dialog>
}
