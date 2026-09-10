import { isAuthError } from '@/lib/session'
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

interface Subscriber {
  refresh: (event: ProjectUpdate) => Promise<unknown>
  connection: (ready: boolean) => void
  error: (error: unknown) => void
}
const transports = new Map<string, { subscribers: Set<Subscriber>; ready: boolean; error?: unknown; dispose: () => void }>()
function subscribeTransport(token: string, project: string, subscriber: Subscriber) {
  const key = JSON.stringify([token, project])
  let transport = transports.get(key)
  if (!transport) {
    const subscribers = new Set<Subscriber>()
    let stopped = false
    let socket: WebSocket | undefined
    let timer: ReturnType<typeof setTimeout> | undefined
    let failures = 0
    const current = { subscribers, ready: false, error: undefined as unknown, dispose: () => {
      stopped = true
      clearTimeout(timer)
      socket?.close()
    } }
    transport = current
    transports.set(key, current)
    const ready = (value: boolean) => {
      if (current.ready === value) return
      current.ready = value
      subscribers.forEach(listener => listener.connection(value))
    }
    const retry = () => {
      if (stopped) return
      ready(false)
      const delay = Math.min(30000, 2000 * 2 ** Math.min(failures++, 4))
      timer = setTimeout(() => void connect(), delay + Math.random() * delay * 0.2)
    }
    const connect = async () => {
      try {
        const session = await api<SyncSession>(token, `/projects/${encodeURIComponent(project)}/session`, { method: 'POST' })
        if (stopped) return
        const next = new WebSocket(`${syncBase(session)}/${encodeURIComponent(session.room)}?ticket=${encodeURIComponent(session.token)}`)
        socket = next
        next.onmessage = message => {
          if (stopped || socket !== next) return
          const event = parseProjectUpdate(message.data)
          if (!event) return
          failures = 0
          ready(true)
          subscribers.forEach(listener => { void listener.refresh(event).catch(() => {}) })
        }
        next.onclose = () => { if (socket === next) { socket = undefined; retry() } }
        next.onerror = () => { if (!stopped && socket === next) ready(false) }
      } catch (error) {
        if (stopped) return
        ready(false)
        if (isAuthError(error) || (error as { status?: number })?.status === 403) {
          current.error = error
          subscribers.forEach(listener => listener.error(error))
        } else retry()
      }
    }
    // Register the first subscriber before a synchronous mocked failure settles.
    queueMicrotask(() => { if (!stopped) void connect() })
  }
  transport.subscribers.add(subscriber)
  subscriber.connection(transport.ready)
  const current = transport
  queueMicrotask(() => {
    if (!current.subscribers.has(subscriber)) return
    if (current.error) subscriber.error(current.error)
    else if (current.ready) void subscriber.refresh({ type: 'refresh' }).catch(() => {})
  })
  return () => {
    transport!.subscribers.delete(subscriber)
    if (!transport!.subscribers.size) {
      transport!.dispose()
      transports.delete(key)
    }
  }
}

/** One transport per credential/project, with per-consumer invalidation coalescing. */
export function useProjectUpdates(token: string, project: string, refresh: (event?: ProjectUpdate) => Promise<unknown>, onError?: (error: unknown) => void) {
  const shared = useContext(ProjectUpdatesContext)
  const [connection, setConnection] = useState<{ scope: string; ready: boolean } | null>(null)
  const scope = JSON.stringify([token, project])
  const onRefresh = useEffectEvent(refresh)
  const onFailure = useEffectEvent((error: unknown) => onError?.(error))
  useEffect(() => {
    if (!token || !project) return
    let stopped = false
    let refreshing = false
    let pending: ProjectUpdate | null = null
    const update = async (event: ProjectUpdate = { type: 'refresh' }) => {
      pending = !pending ? event : (!pending.topics || !event.topics ? { type: 'refresh' } : { type: 'refresh', topics: [...new Set([...pending.topics, ...event.topics])] })
      if (refreshing) return
      refreshing = true
      while (pending && !stopped) {
        const next = pending
        pending = null
        try { await onRefresh(next) } catch { /* owning page renders errors */ }
      }
      refreshing = false
    }
    const unsubscribe = shared?.project === project
      ? shared.subscribe(update)
      : subscribeTransport(token, project, { refresh: update, connection: ready => setConnection({ scope, ready }), error: onFailure })
    return () => { stopped = true; unsubscribe() }
  }, [token, project, shared, scope])
  return shared?.project === project ? shared.connected === true : connection?.scope === scope && connection.ready
}
