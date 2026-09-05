import { useCallback } from 'react'
import { useLocation } from 'react-router'
import { useWorkspaceNavigate } from './useWorkspace'
export function useWorkspaceSection(): [string | null, (id: string | null) => void] {
  const location = useLocation()
  const navigate = useWorkspaceNavigate()
  const section = new URLSearchParams(location.search).get('section')
  const select = useCallback(
    (id: string | null) => {
      const search = new URLSearchParams(location.search)
      if ((search.get('section') ?? null) === id) return
      if (id) search.set('section', id)
      else search.delete('section')
      search.delete('check')
      void navigate({ search: search.toString() }, { replace: true })
    },
    [location.search, navigate],
  )
  return [section, select]
}
