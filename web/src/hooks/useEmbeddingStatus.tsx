import { affectsProjectTopic, ProjectUpdatesContext, useProjectUpdates } from './useProjectUpdates'
import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import { searchStatus, syncSearch, type SearchStatus } from '@/lib/hybrid-search'
import type { ServerSync } from '@/lib/save-status'

export const STATUS_POLL_OPEN_MS = 3000
export const STATUS_POLL_IDLE_MS = 20000

interface EmbeddingStatusValue {
  status: SearchStatus | null
  error: string
  syncing: boolean
  deferred: boolean
  localPending: boolean
  awaitingFreshStatus: boolean
  embed: () => Promise<void>
  clearNotice: () => void
  /** A dialog showing progress: read now, then every few seconds until the returned release is called. */
  watch: () => () => void
}
const EmbeddingStatusContext = createContext<EmbeddingStatusValue | null>(null)
export const useEmbeddingStatus = () => useContext(EmbeddingStatusContext)

/** One status reader per document. Neither the search query nor query results are cached here. */
export function EmbeddingStatusProvider({ token, map, server = 'current', children }: {
  token: string; map: string; server?: ServerSync; children: ReactNode
}) {
  const [status, setStatus] = useState<SearchStatus | null>(null)
  const [error, setError] = useState('')
  const [syncing, setSyncing] = useState(false)
  const [deferred, setDeferred] = useState(false)
  const [freshAfterSave, setFreshAfterSave] = useState(false)
  const request = useRef<AbortController | null>(null)
  const syncingRef = useRef(false)
  const revision = useRef(0)
  const watchers = useRef(0)
  const refresh = useRef<() => void>(() => {})
  const schedule = useRef<() => void>(() => {})
  const localPending = server === 'behind'
  const pending = useRef(localPending)
  const shared = useContext(ProjectUpdatesContext)
  const liveTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const connected = useProjectUpdates(token, shared?.project ?? '', async event => {
    if (!affectsProjectTopic(event, 'search')) return
    if (!liveTimer.current) liveTimer.current = setTimeout(() => {
      liveTimer.current = undefined
      refresh.current()
    }, 1000)
  })
  const live = useRef(connected)
  live.current = connected
  useEffect(() => { schedule.current() }, [connected])
  useEffect(() => () => { clearTimeout(liveTimer.current); liveTimer.current = undefined }, [token, map])
  useEffect(() => {
    let stopped = false
    let timer: ReturnType<typeof setTimeout> | undefined
    setFreshAfterSave(false)
    let failed = false
    const later = () => {
      clearTimeout(timer)
      if (stopped || document.visibilityState === 'hidden' || (watchers.current === 0 && live.current && !failed)) return
      timer = setTimeout(read, watchers.current > 0 ? STATUS_POLL_OPEN_MS : STATUS_POLL_IDLE_MS)
    }
    const read = () => {
      if (stopped || document.visibilityState === 'hidden') return
      if (syncingRef.current) { later(); return }
      const controller = new AbortController()
      request.current = controller
      const ownsRequest = () => !stopped && !controller.signal.aborted && request.current === controller
      searchStatus(token, map, controller.signal).then(value => {
        if (!ownsRequest()) return
        failed = false
        setStatus(value); setError(''); setFreshAfterSave(!pending.current)
        if (value.projection === 'current') setDeferred(false)
      }).catch((reason: Error) => {
        if (ownsRequest()) { failed = true; setError(reason.message); setFreshAfterSave(false) }
      }).finally(() => {
        // An aborted read may settle after its replacement. Only the current
        // owner may release the slot or schedule the next poll.
        if (ownsRequest()) { request.current = null; later() }
      })
    }
    const now = () => { clearTimeout(timer); setFreshAfterSave(false); request.current?.abort(); read() }
    const visibility = () => {
      if (document.visibilityState === 'visible') now()
      else { clearTimeout(timer); request.current?.abort(); request.current = null }
    }
    refresh.current = now
    schedule.current = later
    document.addEventListener('visibilitychange', visibility)
    read()
    return () => {
      stopped = true
      refresh.current = () => {}
      schedule.current = () => {}
      request.current?.abort()
      clearTimeout(timer)
      document.removeEventListener('visibilitychange', visibility)
    }
  }, [token, map])
  useEffect(() => {
    const was = pending.current
    pending.current = localPending
    if (localPending) revision.current++
    else if (was) refresh.current()
  }, [localPending])
  const watch = useCallback(() => {
    watchers.current++
    refresh.current()
    return () => { watchers.current-- }
  }, [])
  const clearNotice = useCallback(() => setDeferred(false), [])
  const embed = useCallback(async () => {
    if (syncingRef.current || localPending || !status?.configured) return
    syncingRef.current = true
    setSyncing(true); setError(''); setDeferred(false)
    // A poll begun before this action cannot overwrite the action's response.
    request.current?.abort()
    request.current = null
    const startedAt = revision.current
    try {
      const value = await syncSearch(token, map)
      setStatus(value); setDeferred(value.sync === 'deferred')
      setFreshAfterSave(startedAt === revision.current)
    } catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)) }
    finally { syncingRef.current = false; setSyncing(false); schedule.current() }
  }, [token, map, localPending, status?.configured])
  return <EmbeddingStatusContext value={{ status, error, syncing, deferred, localPending, awaitingFreshStatus: !freshAfterSave || server === 'unknown', embed, clearNotice, watch }}>{children}</EmbeddingStatusContext>
}
