import { useEffect, useEffectEvent, useState } from 'react'
import * as Y from 'yjs'
import { WebsocketProvider } from 'y-websocket'
import { syncBase } from '@/lib/collab-session'
import { renewSession } from '@/lib/renew-session'
import { loadToken } from '@/lib/session'
export type SyncConnection = { ydoc: Y.Doc; provider: WebsocketProvider }
export function useSyncConnection(session: { object: string; room: string; token: string; expires_at: string; url: string }, onError: (error: unknown) => void) {
 const reportError = useEffectEvent(onError)
 const [connection,setConnection]=useState<SyncConnection|null>(null)
 useEffect(()=>{
  const ydoc=new Y.Doc()
  const provider=new WebsocketProvider(syncBase(session),session.room,ydoc,{params:{ticket:session.token},connect:false,disableBc:true})
  setConnection({ydoc,provider})
  const stop=renewSession(loadToken(),`/mindmaps/${encodeURIComponent(session.object)}/session`,session,provider,error => reportError(error))
  return ()=>{stop();provider.destroy();ydoc.destroy()}
 },[session])
 return connection
}
