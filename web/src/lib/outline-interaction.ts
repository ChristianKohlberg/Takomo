import { flattenSections, type PlanSection } from './plan-sections'
import type { SectionPlacement } from './plan-structure'
/** The middle of a row intentionally reparents; the edges preserve sibling placement. */
export function outlineDropPlacement(y: number, top: number, height: number): SectionPlacement {
  const fraction = height > 0 ? (y - top) / height : 0.5
  return fraction < 0.25 ? 'before' : fraction > 0.75 ? 'after' : 'child'
}
/** Validate against the latest tree; a remote edit can invalidate a drag in flight. */
export function validOutlineDrop(sections: readonly PlanSection[], source: string, target: string): boolean {
  const all = flattenSections(sections)
  const from = all.find(section => section.key === source)
  return !!from && all.some(section => section.key === target) && !flattenSections([from]).some(section => section.key === target)
}
export function outlineParent(sections: readonly PlanSection[], key: string): string | null {
  for (const section of sections) {
    if (section.children.some(child => child.key === key)) return section.key
    const parent = outlineParent(section.children, key)
    if (parent) return parent
  }
  return null
}
