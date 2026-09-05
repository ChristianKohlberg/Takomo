import { renewSession } from '@/lib/renew-session'
import { useEffect, useEffectEvent, useState } from 'react'
import * as Y from 'yjs'
import { WebsocketProvider } from 'y-websocket'
import { api } from '@/lib/api'
import { syncBase, type SyncSession } from '@/lib/collab-session'

export interface Collaboration {
  doc: Y.Doc
  provider: WebsocketProvider
  session: SyncSession
}
export type SyncState = 'connecting' | 'live' | 'reconnecting'

/** Each committed effect owns exactly one replica and one socket. */
export function useCollaboration(token: string, path: string | null, onError: (error: unknown) => void) {
  const reportError = useEffectEvent(onError)
  const [client, setClient] = useState<Collaboration | null>(null)
  const [state, setState] = useState<SyncState>('connecting')
  const [ready, setReady] = useState(false)
  const [peers, setPeers] = useState<{ name: string; node?: string; color: string }[]>([])
  useEffect(() => {
    setClient(null)
    setReady(false)
    setPeers([])
    setState('connecting')
    if (!token || !path) return
    let disposed = false
    let cleanup: (() => void) | undefined
    const abort = new AbortController()
    void api<SyncSession>(token, path, { method: 'POST', signal: abort.signal }).then(session => {
      if (disposed) return
      const doc = new Y.Doc()
      const provider = new WebsocketProvider(syncBase(session), session.room, doc, {
        params: { ticket: session.token }, connect: false, disableBc: true,
      })
      const awareness = () => {
        const people: { name: string; node?: string; color: string }[] = []
        provider.awareness.getStates().forEach((value, id) => {
          if (id !== provider.awareness.clientID && typeof value.user?.name === 'string') {
            people.push({ name: value.user.name, color: value.user.color ?? '#1f4e78', node: value.node })
          }
        })
        setPeers(people)
      }
      const status = ({ status }: { status: string }) => {
        setState(status === 'connected' ? (provider.synced ? 'live' : 'connecting') : 'reconnecting')
      }
      const sync = (synced: boolean) => { if (synced) { setReady(true); setState('live') } }
      provider.on('status', status)
      provider.on('sync', sync)
      provider.awareness.on('change', awareness)
      provider.awareness.setLocalStateField('user', { name: session.display, color: '#1f4e78' })
      setClient({ doc, provider, session })
      provider.connect()
      const stopRenewal = renewSession(token, path, session, provider, error => reportError(error))
      cleanup = () => {
        stopRenewal()
        provider.off('status', status)
        provider.off('sync', sync)
        provider.awareness.off('change', awareness)
        provider.destroy()
        doc.destroy()
      }
    }).catch(error => { if (!disposed) reportError(error) })
    return () => { disposed = true; abort.abort(); cleanup?.() }
  }, [token, path])
  return { client, state, ready, peers }
}
