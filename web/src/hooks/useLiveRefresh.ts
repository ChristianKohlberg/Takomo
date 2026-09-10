import { isAuthError } from '@/lib/session'
import { useEffect, useEffectEvent, useRef } from 'react'
import { affectsProjectTopic, useProjectUpdates, type ProjectTopic } from './useProjectUpdates'

interface Options {
  token: string
  project: string
  /** Changes discard an obsolete request and fetch the new resource. */
  scope: string
  topics: ProjectTopic[]
  load: (signal: AbortSignal) => Promise<unknown>
  onError?: (error: unknown) => void
  enabled?: boolean
  paused?: boolean
  automatic?: boolean
  activeMs?: number | false
  fallbackMs?: number
  minIntervalMs?: number
}

/** Visible-only refreshes. Events, active polling and recovery share one request slot. */
export function useLiveRefresh({ token, project, scope, topics, load, onError, enabled = true, paused = false, automatic = true, activeMs = false, fallbackMs = 30000, minIntervalMs = 1000 }: Options) {
  const invalidate = useRef<() => void>(() => {})
  const terminal = useRef(false)
  const authScope = useRef('')
  const identity = JSON.stringify([token, project])
  if (authScope.current !== identity) { authScope.current = identity; terminal.current = false }
  const failure = useEffectEvent((error: unknown) => onError?.(error))
  const connected = useProjectUpdates(enabled ? token : '', project, async event => {
    if (automatic && affectsProjectTopic(event, ...topics)) invalidate.current()
  }, error => { terminal.current = true; onError?.(error) })
  const read = useEffectEvent(load)
  const options = useRef({ connected, paused, automatic, activeMs, fallbackMs, minIntervalMs })
  options.current = { connected, paused, automatic, activeMs, fallbackMs, minIntervalMs }
  const wake = useRef<() => void>(() => {})
  useEffect(() => {
    if (!enabled || !token) return
    let stopped = false
    let dirty = true
    let request: AbortController | null = null
    let timer: ReturnType<typeof setTimeout> | undefined
    let lastStarted = -Infinity
    let failed = false
    let wasAutomatic = options.current.automatic
    const visible = () => document.visibilityState !== 'hidden'
    const schedule = () => {
      clearTimeout(timer)
      if (stopped || terminal.current || !visible() || options.current.paused || request) return
      const current = options.current
      const interval = current.automatic ? (failed ? current.fallbackMs : current.activeMs || (!current.connected ? current.fallbackMs : false)) : false
      if (!dirty && !interval) return
      const delay = dirty ? Math.max(0, lastStarted + current.minIntervalMs - Date.now()) : interval as number
      timer = setTimeout(() => { dirty = true; void run() }, delay)
    }
    const run = async () => {
      if (stopped || !visible() || options.current.paused || terminal.current || request) return
      dirty = false
      lastStarted = Date.now()
      const controller = new AbortController()
      request = controller
      try { await read(controller.signal); if (request === controller && !controller.signal.aborted) failed = false }
      catch (error) {
        if (request === controller && !controller.signal.aborted && !stopped) {
          failed = true
          if (isAuthError(error)) terminal.current = true
          failure(error)
        }
      }
      finally {
        if (request === controller) { request = null; schedule() }
      }
    }
    const mark = () => { dirty = true; schedule() }
    const visibility = () => {
      if (visible()) { if (options.current.automatic) mark(); else schedule() }
      else clearTimeout(timer)
    }
    invalidate.current = mark
    wake.current = () => {
      if (options.current.automatic && !wasAutomatic) dirty = true
      wasAutomatic = options.current.automatic
      if (options.current.paused && request) {
        request.abort()
        request = null
        dirty = true
      }
      schedule()
    }
    document.addEventListener('visibilitychange', visibility)
    const focus = () => { if (visible() && options.current.automatic) mark() }
    window.addEventListener('focus', focus)
    if (visible() && !options.current.paused) void run()
    return () => {
      stopped = true
      request?.abort()
      clearTimeout(timer)
      invalidate.current = () => {}
      wake.current = () => {}
      document.removeEventListener('visibilitychange', visibility)
      window.removeEventListener('focus', focus)
    }
  }, [token, project, scope, enabled])
  useEffect(() => { wake.current() }, [connected, paused, automatic, activeMs, fallbackMs, minIntervalMs])
  return { connected, refresh: () => invalidate.current() }
}
