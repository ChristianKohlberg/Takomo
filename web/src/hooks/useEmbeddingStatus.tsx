import { createContext, useCallback, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import { searchStatus, syncSearch, type SearchStatus } from '@/lib/hybrid-search'

interface EmbeddingStatusValue {
  status: SearchStatus | null
  error: string
  syncing: boolean
  deferred: boolean
  localPending: boolean
  awaitingFreshStatus: boolean
  embed: () => Promise<void>
  clearNotice: () => void
}
const EmbeddingStatusContext = createContext<EmbeddingStatusValue | null>(null)
export const useEmbeddingStatus = () => useContext(EmbeddingStatusContext)

/** One status reader per document. Neither the search query nor query results are cached here. */
export function EmbeddingStatusProvider({ token, map, localPending = false, children }: {
  token: string; map: string; localPending?: boolean; children: ReactNode
}) {
  const [status, setStatus] = useState<SearchStatus | null>(null)
  const [error, setError] = useState('')
  const [syncing, setSyncing] = useState(false)
  const [deferred, setDeferred] = useState(false)
  const [freshAfterSave, setFreshAfterSave] = useState(false)
  const request = useRef<AbortController | null>(null)
  const syncingRef = useRef(false)
  const revision = useRef(0)
  useEffect(() => { if (localPending) revision.current++ }, [localPending])
  useEffect(() => {
    let stopped = false
    let timer: ReturnType<typeof setTimeout>
    setFreshAfterSave(false)
    const read = () => {
      if (syncingRef.current) { timer = setTimeout(read, 3000); return }
      const controller = new AbortController()
      request.current = controller
      searchStatus(token, map, controller.signal).then(value => {
        if (controller.signal.aborted) return
        setStatus(value); setError(''); setFreshAfterSave(!localPending)
        if (value.projection === 'current') setDeferred(false)
      }).catch((reason: Error) => {
        if (!controller.signal.aborted) { setError(reason.message); setFreshAfterSave(false) }
      }).finally(() => { if (!stopped) timer = setTimeout(read, 3000) })
    }
    read()
    return () => { stopped = true; request.current?.abort(); clearTimeout(timer) }
  }, [token, map, localPending])
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
  return <EmbeddingStatusContext value={{ status, error, syncing, deferred, localPending, awaitingFreshStatus: !freshAfterSave, embed, clearNotice }}>{children}</EmbeddingStatusContext>
}
