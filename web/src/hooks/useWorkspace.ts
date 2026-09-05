import { useCallback, useEffect } from 'react'
import { useLocation, useNavigate, type NavigateFunction, type NavigateOptions, type To } from 'react-router'
import { loadProject, saveProject } from '@/lib/session'

const paths = new Set(['/mindmaps', '/documents', '/verification'])

/** Explicit URL scope wins over the last project visited in another tab. */
export function useWorkspaceProject(): [string, (id: string) => void] {
  const location = useLocation()
  const navigate = useNavigate()
  const project = new URLSearchParams(location.search).get('project') ?? loadProject()
  useEffect(() => {
    if (!project) return
    saveProject(project)
    const search = new URLSearchParams(location.search)
    if (!search.has('project')) {
      search.set('project', project)
      void navigate({ search: search.toString(), hash: location.hash }, { replace: true })
    }
  }, [project, location.search, location.hash, navigate])
  const select = useCallback((id: string) => {
    saveProject(id)
    const search = new URLSearchParams(location.search)
    search.set('project', id)
    void navigate({ search: search.toString(), hash: '' })
  }, [location.search, navigate])
  return [project, select]
}

/** Keep scope on links between plan views and on hash-only navigation. */
export function useWorkspaceNavigate(): NavigateFunction {
  const location = useLocation()
  const navigate = useNavigate()
  return useCallback((to: To | number, options?: NavigateOptions) => {
    if (typeof to === 'number') return navigate(to)
    const url = typeof to === 'string' ? new URL(to, window.location.origin + location.pathname + location.search) : null
    const target = url ? { pathname: url.pathname, search: url.search, hash: url.hash } : { ...(to as Exclude<To, string>) }
    if (paths.has(target.pathname ?? location.pathname)) {
      const search = new URLSearchParams(target.search ?? location.search)
      if (!search.has('project')) search.set('project', new URLSearchParams(location.search).get('project') ?? loadProject())
      target.search = search.toString()
    }
    return navigate(target, options)
  }, [location.pathname, location.search, navigate]) as NavigateFunction
}
