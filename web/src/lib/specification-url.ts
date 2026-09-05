export type SpecificationView = 'document' | 'map' | 'tests'
export function specificationPath(project: string): string {
  return project ? `/projects/${encodeURIComponent(project)}/specification` : '/specification'
}
export function specificationLink(
  project: string,
  view: SpecificationView = 'document',
  section?: string | null,
): string {
  const query = new URLSearchParams({ view })
  if (section) query.set('section', section)
  return `${specificationPath(project)}?${query}`
}
export function specificationProject(path: string): string | null {
  const match = /^\/projects\/([^/]+)\/specification\/?$/.exec(path)
  if (!match) return null
  try {
    return decodeURIComponent(match[1]!)
  } catch {
    return null
  }
}
export function specificationView(search: string): SpecificationView {
  const view = new URLSearchParams(search).get('view')
  return view === 'map' || view === 'tests' ? view : 'document'
}
export const legacyViews: Record<string, SpecificationView> = {
  '/documents': 'document',
  '/mindmaps': 'map',
  '/verification': 'tests',
}
