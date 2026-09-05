import { useEffect, useEffectEvent, useState } from 'react'
import * as Y from 'yjs'
import { WebsocketProvider } from 'y-websocket'
import { syncBase } from '@/lib/collab-session'
import { renewSession } from '@/lib/renew-session'
import { loadToken } from '@/lib/session'
import { trackSave, type SaveSession, type SaveState } from '@/lib/save-status'
export type SyncConnection = { ydoc: Y.Doc; provider: WebsocketProvider }
export function useSyncConnection(session: SaveSession & { room: string; token: string; expires_at: string; url: string }, onError: (error: unknown) => void, onSave?: (state: SaveState) => void) {
  const reportError = useEffectEvent(onError)
  const reportSave = useEffectEvent((state: SaveState) => onSave?.(state))
  const [connection, setConnection] = useState<SyncConnection | null>(null)
  useEffect(() => {
    let disposed = false
    setConnection(null)
    const ydoc = new Y.Doc()
    const provider = new WebsocketProvider(syncBase(session), session.room, ydoc, { params: { ticket: session.token }, connect: false, disableBc: true })
    const tracker = trackSave(ydoc, provider, session, state => reportSave(state))
    void tracker.ready.then(() => { if (!disposed) setConnection({ ydoc, provider }) })
    const stop = renewSession(loadToken(), `/mindmaps/${encodeURIComponent(session.object)}/session`, session, provider, error => reportError(error))
    return () => { disposed = true; stop(); tracker.destroy(); provider.destroy(); ydoc.destroy() }
  }, [session])
  return connection
}
