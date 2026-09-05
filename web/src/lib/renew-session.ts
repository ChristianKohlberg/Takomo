import type { WebsocketProvider } from 'y-websocket'
import { api } from './api'
import type { SyncSession } from './collab-session'

/** Rotate expiring credentials without replacing the replica or losing edits. */
export function renewSession(token: string, path: string, session: Pick<SyncSession, 'expires_at'>,
  provider: WebsocketProvider, onError: (error: unknown) => void) {
  let stopped = false
  let timer: ReturnType<typeof setTimeout>
  const schedule = (expires: string) => {
    const remaining = Date.parse(expires) - Date.now() - 60_000
    if (!Number.isFinite(remaining)) return
    timer = setTimeout(() => { void renew() }, Math.max(1000, remaining))
  }
  const renew = async () => {
    try {
      const next = await api<SyncSession>(token, path, { method: 'POST' })
      if (stopped) return
      provider.params.ticket = next.token
      provider.disconnect()
      provider.connect()
      schedule(next.expires_at)
    } catch (error) {
      if (!stopped) {
        onError(error)
        timer = setTimeout(() => { void renew() }, 30_000)
      }
    }
  }
  schedule(session.expires_at)
  return () => { stopped = true; clearTimeout(timer) }
}
