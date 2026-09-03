// The plan as a sequence of numbered sections.
//
// A project has ONE plan, and the map and the document are two renderings of it
// (`spec/one-model-two-views.md`). A node is a section: its title is the
// heading, its depth is the heading level, and tree order is reading order. So
// everything the document view needs about structure is derived here, from the
// same tree the canvas draws, and nothing about a section's number, level or
// order is stored anywhere.
//
// This replaces `document-outline.ts`, which folded FOLDERS and documents into
// sections because the plan used to be converted into a tree of document rows.
// The conversion is gone, so the folder model went with it — but the three
// things that module got right are kept intact and still tested here: the
// number is computed from position, a fold is per-viewer, and jumping to a
// section unfolds whatever hides it.
//
// Pure, and importing nothing but a type: the numbering rules are worth testing
// without a document, a socket or a canvas that jsdom could not lay out anyway.

/** The four fields the plan's shape is made of. See `readPlanTree`. */
export interface PlanNode {
  id: string
  parent: string | null
  order: string
  title: string
  /** Integer sibling rank, assigned by `normaliseNodes`. */
  position: number
}

/** One section of the plan. */
export interface PlanSection {
  /** The node id. A section IS a node. */
  key: string
  /** Position address, `2.1.3`. Derived from the tree, never stored. */
  number: string
  /** 0 for a first-ring node — the plan's H1s. */
  depth: number
  title: string
  children: PlanSection[]
}

/**
 * The plan's sections, in reading order.
 *
 * `nodes` is expected to arrive already repaired and sibling-ordered (that is
 * what `normaliseNodes` is for), so this walks rather than sorts. A node whose
 * parent is not in the set is treated as a first-ring node rather than dropped:
 * losing a whole branch of somebody's plan because its parent arrived in a later
 * update is worse than showing it at the top.
 */
export function planSections(nodes: readonly PlanNode[]): PlanSection[] {
  const known = new Set(nodes.map((n) => n.id))
  const kids = new Map<string | null, PlanNode[]>()
  for (const n of nodes) {
    const parent = n.parent !== null && known.has(n.parent) ? n.parent : null
    const list = kids.get(parent)
    if (list) list.push(n)
    else kids.set(parent, [n])
  }

  const seen = new Set<string>()
  const build = (parent: string | null, depth: number, prefix: string): PlanSection[] => {
    const out: PlanSection[] = []
    for (const n of kids.get(parent) ?? []) {
      // `normaliseNodes` has already broken every cycle; this walks whatever it
      // is handed, and a guard costs one Set rather than a hung render.
      if (seen.has(n.id)) continue
      seen.add(n.id)
      const number = prefix ? `${prefix}.${out.length + 1}` : String(out.length + 1)
      out.push({
        key: n.id,
        number,
        depth,
        title: n.title.trim(),
        children: build(n.id, depth + 1, number),
      })
    }
    return out
  }

  return build(null, 0, '')
}

/** Every section in reading order, parents before their children. */
export function flattenSections(sections: readonly PlanSection[]): PlanSection[] {
  const out: PlanSection[] = []
  const walk = (list: readonly PlanSection[]): void => {
    for (const s of list) {
      out.push(s)
      walk(s.children)
    }
  }
  walk(sections)
  return out
}

/** How many sections sit beneath this one, at any depth. What a folded row says
 *  it is holding back. */
export function sectionCount(section: PlanSection): number {
  let total = 0
  for (const child of section.children) total += 1 + sectionCount(child)
  return total
}

/**
 * The rows a viewer sees, given the sections THEY folded.
 *
 * Fold is per-viewer and browser-local, the same rule the canvas follows:
 * folding a branch must not fold it under somebody else who is reading the same
 * plan.
 */
export function visibleSections(
  sections: readonly PlanSection[],
  collapsed: ReadonlySet<string>,
): PlanSection[] {
  const out: PlanSection[] = []
  const walk = (list: readonly PlanSection[]): void => {
    for (const s of list) {
      out.push(s)
      if (!collapsed.has(s.key)) walk(s.children)
    }
  }
  walk(sections)
  return out
}

/** Every section key on the path down to `key`, excluding it — what has to be
 *  unfolded for a section to be on screen at all. */
export function ancestorKeys(sections: readonly PlanSection[], key: string): string[] {
  const path: string[] = []
  const walk = (list: readonly PlanSection[], trail: string[]): boolean => {
    for (const s of list) {
      if (s.key === key) {
        path.push(...trail)
        return true
      }
      if (walk(s.children, [...trail, s.key])) return true
    }
    return false
  }
  walk(sections, [])
  return path
}

/**
 * Is this the same plan, structurally?
 *
 * The document view holds the tree in React state and rebuilds it whenever the
 * shared document changes — which, with prose living inside the nodes, is every
 * character anybody types. Nothing about the outline changes when a paragraph
 * does, so the projection is only replaced when this says the shape moved. Not a
 * micro-optimisation: without it every remote keystroke re-renders every
 * section, and a section is a mounted editor.
 */
export function sameTree(a: readonly PlanNode[], b: readonly PlanNode[]): boolean {
  if (a.length !== b.length) return false
  for (let i = 0; i < a.length; i += 1) {
    const x = a[i] as PlanNode
    const y = b[i] as PlanNode
    if (x.id !== y.id || x.parent !== y.parent || x.title !== y.title || x.order !== y.order) {
      return false
    }
  }
  return true
}
