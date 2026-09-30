// Section focus: the document view narrowed to one section and its subtree.
//
// A local projection, like the map's branch focus (`mindmap-focus.ts`): nothing
// here writes to the shared document. The focused sections keep their REAL
// numbers and depths — the projection filters, it never renumbers — so "7.4.2"
// in focus is the same "7.4.2" a colleague sees in the whole document.

import { flattenSections, type PlanSection } from './plan-sections'

export interface SectionFocusScope {
  /** The focused section, with its real number, depth and children. */
  root: PlanSection
  /** Its ancestors, outermost first — the breadcrumb between the document and it. */
  ancestors: PlanSection[]
  /** The root and every section beneath it. */
  ids: ReadonlySet<string>
}

/** The focus scope for `root`, or null when there is no such section (unknown or deleted id). */
export function sectionFocusScope(sections: readonly PlanSection[], root: string | null | undefined): SectionFocusScope | null {
  if (!root) return null
  const walk = (list: readonly PlanSection[], trail: PlanSection[]): SectionFocusScope | null => {
    for (const section of list) {
      if (section.key === root) return { root: section, ancestors: trail, ids: new Set(flattenSections([section]).map(s => s.key)) }
      const found = walk(section.children, [...trail, section])
      if (found) return found
    }
    return null
  }
  return walk(sections, [])
}

/** Is `key` shown under this scope? No scope shows everything. */
export function inFocus(scope: SectionFocusScope | null, key: string): boolean {
  return !scope || scope.ids.has(key)
}

/** Work waiting outside the focus, so narrowing the view never hides that it exists. */
export function outsideFocus(
  scope: SectionFocusScope,
  known: ReadonlySet<string>,
  pending: Readonly<Record<string, number>>,
  threads: readonly { sectionId: string; resolved: boolean }[],
): { proposals: number; comments: number } {
  let proposals = 0
  for (const [key, count] of Object.entries(pending)) if (known.has(key) && !scope.ids.has(key)) proposals += count
  let comments = 0
  for (const thread of threads) if (!thread.resolved && known.has(thread.sectionId) && !scope.ids.has(thread.sectionId)) comments += 1
  return { proposals, comments }
}
