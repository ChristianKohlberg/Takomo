export type SpecificationView = 'document' | 'map' | 'tests'
export function specificationPath(project: string): string {
  return project ? `/projects/${encodeURIComponent(project)}/specification` : '/specification'
}
export function specificationLink(
  project: string,
  view: SpecificationView = 'document',
  section?: string | null,
  focus?: string | null,
): string {
  const query = new URLSearchParams({ view })
  if (section) query.set('section', section)
  // Section focus is a personal view of the document only: the other views
  // have no subtree to narrow to, so it is never carried into them.
  if (focus && view === 'document') query.set('focus', focus)
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
/** The section a document view is narrowed to (`focus=`), or null. */
export function specificationFocus(search: string): string | null {
  return new URLSearchParams(search).get('focus') || null
}
