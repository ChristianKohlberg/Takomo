import * as Y from 'yjs'
import * as encoding from 'lib0/encoding'
import * as decoding from 'lib0/decoding'
import type { WebsocketProvider } from 'y-websocket'
import { persistReplica, type LocalSave } from './durable-replica'

export type SaveState = 'connecting' | 'syncing' | 'saved' | 'offline' | 'local-error' | 'server-error' | 'read-only'
export interface SaveSession { object: string; actor?: string; display?: string; can_write?: boolean; durability_ack?: boolean }

/** A socket handshake is not a save. Only an ordered server durability reply is. */
export function trackSave(doc: Y.Doc, provider: WebsocketProvider, session: SaveSession, changed: (state: SaveState) => void) {
  let local: LocalSave = 'loading'
  let revision = 0
  let acknowledged = -1
  let refused = false
  let timer: ReturnType<typeof setTimeout> | undefined
  let stopped = false
  let recovering = true
  const writable = session.can_write !== false
  const report = () => {
    if (stopped) return
    changed(!writable ? 'read-only' : local === 'error' ? 'local-error' :
      !provider.wsconnected ? (local === 'saved' ? 'offline' : 'connecting') :
      refused ? 'server-error' : provider.synced && local === 'saved' && acknowledged === revision ? 'saved' : 'syncing')
  }
  const request = () => {
    clearTimeout(timer)
    if (stopped || !provider.synced || !provider.wsconnected || !session.durability_ack) return
    const message = encoding.createEncoder()
    encoding.writeVarUint(message, 4)
    encoding.writeVarUint(message, revision)
    provider.ws?.send(encoding.toUint8Array(message))
  }
  const schedule = () => { clearTimeout(timer); timer = setTimeout(request, 600) }
  const update = (_update: Uint8Array, origin: unknown) => {
    if (origin !== provider) { revision++; refused = false; schedule() }
    report()
  }
  doc.on('update', update)
  const persistence = writable ? persistReplica(doc, `${session.actor ?? session.display ?? 'viewer'}:${session.object}`, state => { local = state; report() }) : null
  provider.messageHandlers[4] = (_encoder, decoder) => {
    const seen = decoding.readVarUint(decoder)
    const saved = decoding.readVarUint(decoder) === 1
    if (seen === revision) {
      acknowledged = saved ? seen : -1
      refused = !saved
      if (!saved) { clearTimeout(timer); timer = setTimeout(request, 5000) }
    }
    report()
  }
  const sync = (synced: boolean) => { if (synced) schedule(); report() }
  const status = () => { if (!provider.wsconnected) acknowledged = -1; report() }
  provider.on('sync', sync)
  provider.on('status', status)
  const unload = (event: BeforeUnloadEvent) => {
    if (writable && revision !== acknowledged && (local !== 'saved' || recovering)) { event.preventDefault(); event.returnValue = '' }
  }
  window.addEventListener('beforeunload', unload)
  const ready = (persistence?.ready ?? Promise.resolve()).catch(() => { local = 'error' }).then(() => { recovering = false; if (!writable) local = 'saved'; report() })
  report()
  return {
    ready,
    destroy() {
      stopped = true
      clearTimeout(timer)
      persistence?.destroy()
      doc.off('update', update)
      provider.off('sync', sync)
      provider.off('status', status)
      window.removeEventListener('beforeunload', unload)
    },
  }
}
