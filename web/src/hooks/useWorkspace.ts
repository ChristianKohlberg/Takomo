import { loadProject, saveProject } from '@/lib/session'
import {
  legacyViews,
  specificationLink,
  specificationProject,
  specificationView,
} from '@/lib/specification-url'
import { useCallback, useEffect } from 'react'
import {
  useLocation,
  useNavigate,
  type NavigateFunction,
  type NavigateOptions,
  type To,
} from 'react-router'

export function useWorkspaceProject(): [string, (id: string) => void] {
  const location = useLocation()
  const navigate = useNavigate()
  const project =
    specificationProject(location.pathname) ??
    new URLSearchParams(location.search).get('project') ??
    loadProject()
  useEffect(() => {
    if (project) saveProject(project)
  }, [project])
  const select = useCallback(
    (id: string) => {
      saveProject(id)
      void navigate(specificationLink(id, specificationView(location.search)))
    },
    [location.search, navigate],
  )
  return [project, select]
}

/** Old hand-off helpers also stay inside the mounted specification workspace. */
export function useWorkspaceNavigate(): NavigateFunction {
  const location = useLocation()
  const navigate = useNavigate()
  return useCallback(
    (to: To | number, options?: NavigateOptions) => {
      if (typeof to === 'number') return navigate(to)
      const url =
        typeof to === 'string'
          ? new URL(to, window.location.origin + location.pathname + location.search)
          : null
      const target = url
        ? { pathname: url.pathname, search: url.search, hash: url.hash }
        : { ...(to as Exclude<To, string>) }
      const oldView = target.pathname ? legacyViews[target.pathname] : undefined
      const project =
        specificationProject(location.pathname) ??
        new URLSearchParams(location.search).get('project') ??
        loadProject()
      if (oldView) {
        const search = new URLSearchParams({ view: oldView })
        const hash = new URLSearchParams(target.hash?.slice(1))
        if (hash.get('n')) search.set('section', hash.get('n')!)
        if (hash.get('c')) search.set('check', hash.get('c')!)
        return navigate(
          {
            pathname: specificationLink(project).split('?')[0],
            search: search.toString(),
            hash: '',
          },
          options,
        )
      }
      // Hash-only calls from old check controls become canonical query selections.
      if (target.hash !== undefined && !target.pathname) {
        const search = new URLSearchParams(target.search ?? location.search)
        const hash = new URLSearchParams(target.hash.replace(/^#/, ''))
        for (const [old, key] of [
          ['n', 'section'],
          ['c', 'check'],
        ] as const) {
          if (hash.has(old)) search.set(key, hash.get(old)!)
          else search.delete(key)
        }
        target.search = search.toString()
        target.hash = ''
      }
      return navigate(target, options)
    },
    [location.pathname, location.search, navigate],
  ) as NavigateFunction
}
