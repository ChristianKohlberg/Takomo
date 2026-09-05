import { useCallback } from 'react'
import { useLocation } from 'react-router'
import { useWorkspaceNavigate } from './useWorkspace'
import { readPlanFocus } from '@/lib/plan-url'

export function useWorkspaceSection(): [string | null, (id: string | null) => void] {
  const location = useLocation()
  const navigate = useWorkspaceNavigate()
  const section = readPlanFocus(location.hash)
  const select = useCallback((id: string | null) => {
    const hash = new URLSearchParams(location.hash.slice(1))
    if ((hash.get('n') ?? null) === id) return
    if (id) hash.set('n', id); else hash.delete('n')
    void navigate({ hash: hash.toString() }, { replace: true })
  }, [location.hash, navigate])
  return [section, select]
}
