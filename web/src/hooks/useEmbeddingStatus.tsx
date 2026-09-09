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
  const localPending = server === 'behind'
  useEffect(() => { if (localPending) revision.current++ }, [localPending])
  useEffect(() => {
    let stopped = false
    let timer: ReturnType<typeof setTimeout> | undefined
    setFreshAfterSave(false)
    const later = () => {
      clearTimeout(timer)
      timer = setTimeout(read, watchers.current > 0 ? STATUS_POLL_OPEN_MS : STATUS_POLL_IDLE_MS)
    }
    const read = () => {
      if (stopped || document.visibilityState === 'hidden') return
      if (syncingRef.current) { later(); return }
      const controller = new AbortController()
      request.current = controller
      searchStatus(token, map, controller.signal).then(value => {
        if (controller.signal.aborted) return
        setStatus(value); setError(''); setFreshAfterSave(!localPending)
        if (value.projection === 'current') setDeferred(false)
      }).catch((reason: Error) => {
        if (!controller.signal.aborted) { setError(reason.message); setFreshAfterSave(false) }
      }).finally(() => { if (!stopped) later() })
    }
    const now = () => { clearTimeout(timer); setFreshAfterSave(false); request.current?.abort(); read() }
    const visibility = () => { if (document.visibilityState === 'visible') now() }
    refresh.current = now
    document.addEventListener('visibilitychange', visibility)
    read()
    return () => {
      stopped = true
      refresh.current = () => {}
      request.current?.abort()
      clearTimeout(timer)
      document.removeEventListener('visibilitychange', visibility)
    }
  }, [token, map, localPending])
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
    const startedAt = revision.current
    try {
      const value = await syncSearch(token, map)
      setStatus(value); setDeferred(value.sync === 'deferred')
      setFreshAfterSave(startedAt === revision.current)
    } catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)) }
    finally { syncingRef.current = false; setSyncing(false) }
  }, [token, map, localPending, status?.configured])
  return <EmbeddingStatusContext value={{ status, error, syncing, deferred, localPending, awaitingFreshStatus: !freshAfterSave || server === 'unknown', embed, clearNotice, watch }}>{children}</EmbeddingStatusContext>
}
