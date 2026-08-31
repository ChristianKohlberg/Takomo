// The document tree read as an OUTLINE of one undertaking, not as a filing
// cabinet.
//
// `buildTree` used to group documents into folders and list the two kinds
// separately — folders above, documents below, each alphabetised. That is what a
// file browser does, and it is the wrong shape for what actually lands here now
// that a mindmap converts into documents: `POST /v1/mindmaps/{id}/documents`
// writes a document per thought and files its children UNDER that thought's
// name (`docs/mindmaps.md`), so a branch called `API` becomes a document `API`
// at `Payments rebuild` **and** a folder `Payments rebuild/API`. Rendered as two
// kinds you get a folder `API` sitting next to a document `API`, which reads as
// a bug.
//
// So the model here has ONE kind of thing — a section — and the folder and the
// document that named it are folded into it. Three cases, all of which occur:
//
//   * a document whose title names a sibling folder  → one section, with children
//   * a folder no document names (somebody filed by hand) → a plain group
//   * a document naming no folder                    → a leaf
//
// The invariant that matters is that **every document appears exactly once**.
// Folding claims a document as a folder's head; a claimed document is not also
// listed as a leaf, and an unclaimed one always is. `every_document_appears_
// exactly_once` in the tests is the guard.
//
// The section NUMBER (`1`, `2.1`, `2.1.3`) is computed from position, never
// stored. It is the point of the rail rather than decoration: it gives every
// part of the plan a short shared address, so "§2.1 contradicts §4" is a thing
// somebody can say out loud.
import type { Doc } from './documents'

/** One row of the outline. A document, a folder, or the two folded together. */
export interface OutlineSection {
  /** Stable across reads: the document id where there is one, else the folder path. */
  key: string
  /** Position address, `2.1.3`. Derived from the tree, never stored. */
  number: string
  /** 0 at the top level. */
  depth: number
  title: string
  /** The document this section IS, or null for a folder nothing is filed as. */
  doc: Doc | null
  /** The folder whose contents are this section's children, or null for a leaf. */
  folder: string | null
  children: OutlineSection[]
}

/** `a//b/ ` → `['a','b']`. A path is prose somebody typed, so it is normalised
 *  rather than trusted: an empty segment would otherwise become a nameless
 *  folder that can never be filed into again. */
function segments(path: string): string[] {
  return path
    .split('/')
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
}

/** Title first, id as the tiebreak — so two documents of the same name keep a
 *  fixed order between reads rather than following fetch order. */
function byTitle(a: Doc, b: Doc): number {
  const t = a.title.localeCompare(b.title)
  return t !== 0 ? t : a.id < b.id ? -1 : a.id > b.id ? 1 : 0
}

const nameOf = (folder: string): string => {
  const cut = folder.lastIndexOf('/')
  return cut === -1 ? folder : folder.slice(cut + 1)
}

/**
 * The outline of a project's documents, top level first.
 *
 * Sections at a level are ordered by title, with the key as a tiebreak, and
 * folders are NOT hoisted above documents: once a folder can be a document there
 * is no second kind to sort separately, and interleaving is what makes the
 * numbering read as one sequence.
 */
export function buildOutline(docs: readonly Doc[]): OutlineSection[] {
  /** folder path → the documents filed directly in it. */
  const filed = new Map<string, Doc[]>([['', []]])
  /** folder path → its immediate subfolders. */
  const subfolders = new Map<string, Set<string>>([['', new Set()]])

  const ensure = (folder: string): void => {
    if (filed.has(folder)) return
    filed.set(folder, [])
    subfolders.set(folder, new Set())
    const cut = folder.lastIndexOf('/')
    const parent = cut === -1 ? '' : folder.slice(0, cut)
    ensure(parent)
    subfolders.get(parent)?.add(folder)
  }

  for (const doc of docs) {
    const folder = segments(doc.path).join('/')
    ensure(folder)
    filed.get(folder)?.push(doc)
  }
  for (const list of filed.values()) list.sort(byTitle)

  /** Documents already used as a folder's head. Claiming is what keeps the
   *  every-document-exactly-once invariant true. */
  const claimed = new Set<string>()

  const build = (folder: string, depth: number, prefix: string): OutlineSection[] => {
    const here = filed.get(folder) ?? []
    const kids = [...(subfolders.get(folder) ?? [])].sort((a, b) =>
      nameOf(a).localeCompare(nameOf(b)),
    )

    // Fold first, in a fixed subfolder order, so which of two same-titled
    // documents heads a folder does not depend on iteration luck.
    const heads = new Map<string, Doc>()
    for (const sub of kids) {
      const name = nameOf(sub)
      const head = here.find((d) => d.title === name && !claimed.has(d.id))
      if (head) {
        claimed.add(head.id)
        heads.set(sub, head)
      }
    }

    type Pending = { title: string; key: string; doc: Doc | null; folder: string | null }
    const pending: Pending[] = kids.map((sub) => {
      const head = heads.get(sub) ?? null
      return {
        title: head ? head.title : nameOf(sub),
        key: head ? head.id : sub,
        doc: head,
        folder: sub,
      }
    })
    for (const doc of here) {
      if (claimed.has(doc.id)) continue
      pending.push({ title: doc.title, key: doc.id, doc, folder: null })
    }
    pending.sort((a, b) => {
      const t = a.title.localeCompare(b.title)
      return t !== 0 ? t : a.key < b.key ? -1 : a.key > b.key ? 1 : 0
    })

    return pending.map((p, i) => {
      const number = prefix ? `${prefix}.${i + 1}` : String(i + 1)
      return {
        key: p.key,
        number,
        depth,
        title: p.title,
        doc: p.doc,
        folder: p.folder,
        children: p.folder === null ? [] : build(p.folder, depth + 1, number),
      }
    })
  }

  return build('', 0, '')
}

/** How many sections sit beneath this one, at any depth. What a collapsed row
 *  says it is holding back. */
export function sectionCount(section: OutlineSection): number {
  let total = 0
  for (const child of section.children) total += 1 + sectionCount(child)
  return total
}

/**
 * The rows a viewer sees, given the sections THEY collapsed.
 *
 * Fold is per-viewer and browser-local, the same rule the mindmap follows:
 * collapsing a branch must not collapse it under somebody else who is reading
 * the same plan.
 */
export function visibleSections(
  sections: readonly OutlineSection[],
  collapsed: ReadonlySet<string>,
): OutlineSection[] {
  const out: OutlineSection[] = []
  const walk = (list: readonly OutlineSection[]): void => {
    for (const s of list) {
      out.push(s)
      if (!collapsed.has(s.key)) walk(s.children)
    }
  }
  walk(sections)
  return out
}

/** Every section key on the path down to `key`, excluding it — what has to be
 *  unfolded for a selected document to be on screen at all. */
export function ancestorKeys(
  sections: readonly OutlineSection[],
  key: string,
): string[] {
  const path: string[] = []
  const walk = (list: readonly OutlineSection[], trail: string[]): boolean => {
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
