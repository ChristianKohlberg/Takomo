import { api } from '@/lib/api'
import { syncBase, type SyncSession } from '@/lib/collab-session'
import { createContext, useContext, useEffect, useEffectEvent } from 'react'

export const ProjectUpdatesContext = createContext<{
  project: string
  subscribe: (callback: () => Promise<unknown>) => () => void
} | null>(null)

/** Live invalidation for server-owned lists, verdicts and metadata. */
export function useProjectUpdates(token: string, project: string, refresh: () => Promise<unknown>) {
  const shared = useContext(ProjectUpdatesContext)
  const onRefresh = useEffectEvent(refresh)
  useEffect(() => {
    if (!token || !project) return
    if (shared?.project === project) return shared.subscribe(() => onRefresh())
    let stopped = false
    let socket: WebSocket | undefined
    let timer: ReturnType<typeof setTimeout>
    let refreshing = false
    let again = false
    const update = async () => {
      if (refreshing) {
        again = true
        return
      }
      refreshing = true
      do {
        again = false
        try {
          await onRefresh()
        } catch {
          /* the owning page renders request errors */
        }
      } while (again && !stopped)
      refreshing = false
    }
    const connect = async () => {
      try {
        const session = await api<SyncSession>(
          token,
          `/projects/${encodeURIComponent(project)}/session`,
          { method: 'POST' },
        )
        if (stopped) return
        socket = new WebSocket(
          `${syncBase(session)}/${encodeURIComponent(session.room)}?ticket=${encodeURIComponent(session.token)}`,
        )
        socket.onmessage = () => {
          void update()
        }
        socket.onclose = () => {
          if (!stopped) timer = setTimeout(() => void connect(), 2000)
        }
      } catch {
        if (!stopped) timer = setTimeout(() => void connect(), 2000)
      }
    }
    void connect()
    return () => {
      stopped = true
      clearTimeout(timer)
      socket?.close()
    }
  }, [token, project, shared])
}
