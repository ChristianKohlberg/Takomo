import { useCallback } from 'react'
import { useLocation } from 'react-router'
import { specificationFocus } from '@/lib/specification-url'
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
      search.delete('behavior')
      void navigate({ search: search.toString() }, { replace: true })
    },
    [location.search, navigate],
  )
  return [section, select]
}

export interface FocusChange {
  /** Also move the selection/scroll target (`section=`) in the same entry. */
  section?: string | null
  /** Rewrite the current entry instead of adding one — for a dropped unknown id. */
  replace?: boolean
}

/**
 * Section focus (`focus=`): a personal, shareable narrowing of the document to
 * one section's subtree. Unlike the selection it is a HISTORY entry, so the
 * browser's Back returns to the view that was showing before.
 */
export function useWorkspaceFocus(): [string | null, (id: string | null, change?: FocusChange) => void] {
  const location = useLocation()
  const navigate = useWorkspaceNavigate()
  const focus = specificationFocus(location.search)
  const setFocus = useCallback(
    (id: string | null, change: FocusChange = {}) => {
      const search = new URLSearchParams(location.search)
      if (id) search.set('focus', id)
      else search.delete('focus')
      if (change.section !== undefined) {
        if (change.section) search.set('section', change.section)
        else search.delete('section')
      }
      if (search.toString() === new URLSearchParams(location.search).toString()) return
      search.delete('behavior')
      void navigate({ search: search.toString() }, { replace: change.replace === true })
    },
    [location.search, navigate],
  )
  return [focus, setFocus]
}
