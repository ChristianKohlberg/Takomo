import { api } from '@/lib/api'
import { syncBase, type SyncSession } from '@/lib/collab-session'
import { createContext, useContext, useEffect, useEffectEvent, useState } from 'react'
export const PROJECT_TOPICS = ['document', 'trace', 'checks', 'projects', 'inbox', 'tickets', 'agent', 'history', 'search'] as const
export type ProjectTopic = typeof PROJECT_TOPICS[number]
export interface ProjectUpdate { type: 'refresh'; topics?: ProjectTopic[] }
export const affectsProjectTopic = (event: ProjectUpdate | undefined, ...topics: ProjectTopic[]) => !event?.topics || topics.some(topic => event.topics!.includes(topic))
export function parseProjectUpdate(data: unknown): ProjectUpdate | null {
  try {
    const value = typeof data === 'string' ? JSON.parse(data) : data
    if (!value || value.type !== 'refresh') return null
    if (!Array.isArray(value.topics) || value.topics.some((topic: unknown) => !PROJECT_TOPICS.includes(topic as ProjectTopic))) return { type: 'refresh' }
    return { type: 'refresh', topics: value.topics }
  } catch { return null }
}
export const ProjectUpdatesContext = createContext<{
  project: string
  connected?: boolean
  subscribe: (callback: (event?: ProjectUpdate) => Promise<unknown>) => () => void
} | null>(null)

/** Live invalidation for server-owned lists, verdicts and metadata. */
export function useProjectUpdates(token: string, project: string, refresh: (event?: ProjectUpdate) => Promise<unknown>) {
  const shared = useContext(ProjectUpdatesContext)
  const [connection, setConnection] = useState<{ scope: string; ready: boolean } | null>(null)
  const scope = `${token}:${project}`
  const onRefresh = useEffectEvent(refresh)
  useEffect(() => {
    if (!token || !project) return
    if (shared?.project === project) return shared.subscribe(event => onRefresh(event))
    let stopped = false
    let socket: WebSocket | undefined
    let timer: ReturnType<typeof setTimeout>
    let refreshing = false
    let pending: ProjectUpdate | null = null
    const update = async (event: ProjectUpdate) => {
      pending = !pending ? event : (!pending.topics || !event.topics ? { type: 'refresh' } : { type: 'refresh', topics: [...new Set([...pending.topics, ...event.topics])] })
      if (refreshing) return
      refreshing = true
      while (pending && !stopped) {
        const next = pending
        pending = null
        try { await onRefresh(next) } catch { /* owning page renders request errors */ }
      }
      refreshing = false
    }
    const connect = async () => {
      try {
        const session = await api<SyncSession>(token, `/projects/${encodeURIComponent(project)}/session`, { method: 'POST' })
        if (stopped) return
        socket = new WebSocket(`${syncBase(session)}/${encodeURIComponent(session.room)}?ticket=${encodeURIComponent(session.token)}`)
        socket.onmessage = message => {
          const event = parseProjectUpdate(message.data)
          if (!event || stopped) return
          setConnection({ scope, ready: true })
          void update(event)
        }
        socket.onclose = () => {
          if (!stopped) {
            setConnection({ scope, ready: false })
            timer = setTimeout(() => void connect(), 2000)
          }
        }
        socket.onerror = () => { if (!stopped) setConnection({ scope, ready: false }) }
      } catch {
        if (!stopped) {
          setConnection({ scope, ready: false })
          timer = setTimeout(() => void connect(), 2000)
        }
      }
    }
    void connect()
    return () => {
      stopped = true
      clearTimeout(timer)
      socket?.close()
    }
  }, [token, project, shared, scope])
  return shared?.project === project ? shared.connected === true : connection?.scope === scope && connection.ready
}
