import { useCallback, useEffect, useRef, useState } from 'react'
import { getTrace, type TraceEntry } from '@/lib/mindmaps'
export interface SectionTraceState {
  entries: TraceEntry[]
  loading: boolean
  error: boolean
  dirty: boolean
}
const empty = (): SectionTraceState => ({ entries: [], loading: false, error: false, dirty: true })
/** History is fetched only for disclosed sections. Invalidations retain closed caches as dirty. */
export function useSectionTrace(token: string, map: string | null) {
  const [cache, setCache] = useState<Record<string, SectionTraceState>>({})
  const current = useRef(cache)
  const opened = useRef(new Set<string>())
  const epoch = useRef(0)
  const update = useCallback((id: string, value: SectionTraceState) => {
    current.current = { ...current.current, [id]: value }
    setCache(current.current)
  }, [])
  useEffect(() => {
    const generation = epoch
    generation.current++
    current.current = {}
    setCache({})
    opened.current.clear()
    return () => { generation.current++ }
  }, [token, map])
  const load = useCallback(async (id: string) => {
    if (!token || !map || !opened.current.has(id)) return
    const old = current.current[id] ?? empty()
    if (old.loading || (!old.dirty && !old.error)) return
    const generation = epoch.current
    update(id, { ...old, loading: true, error: false, dirty: false })
    try {
      const page = await getTrace(token, map, { node: id, limit: 500 })
      if (generation !== epoch.current) return
      const dirty = current.current[id]?.dirty ?? false
      update(id, { entries: page.items, loading: false, error: false, dirty })
    } catch {
      if (generation === epoch.current) update(id, { ...current.current[id]!, loading: false, error: true, dirty: true })
    }
  }, [token, map, update])
  useEffect(() => {
    for (const id of opened.current) if (cache[id]?.dirty && !cache[id]?.loading && !cache[id]?.error) void load(id)
  }, [cache, load])
  const setOpen = useCallback((id: string, open: boolean) => {
    if (open) {
      opened.current.add(id)
      void load(id)
    } else {
      opened.current.delete(id)
    }
  }, [load])
  const invalidate = useCallback(() => {
    current.current = Object.fromEntries(Object.entries(current.current).map(([id, value]) => [id, { ...value, dirty: true }]))
    setCache(current.current)
    for (const id of opened.current) void load(id)
  }, [load])
  return { cache, setOpen, invalidate, retry: load }
}
