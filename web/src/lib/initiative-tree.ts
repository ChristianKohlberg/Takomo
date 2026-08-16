// The explorer's folder tree, derived from `metadata.path`.
//
// Initiatives are flat in the store, and deliberately stay that way: `metadata`
// is already a free-form JSON object on every initiative, so a folder is a
// STRING an initiative carries rather than a row anyone has to keep consistent.
// No migration, no orphaned-directory problem, no second thing to delete when a
// document moves — moving one is a `metadata_merge` of its path and nothing else.
//
// The tree itself is reduced from the list on every read and never stored, the
// same rule the `rollup` and the document follow, and for the same reason: a
// cached shape drifts from the rows it describes and nothing notices.

import type { Initiative } from './initiatives'

/** Deepest nesting a path may express. Beyond this, extra segments are joined back. */
export const MAX_DEPTH = 8
/** Longest single folder name. Long enough for a sentence fragment, short enough to render. */
export const MAX_SEGMENT = 48

export interface TreeDoc {
  kind: 'doc'
  /** Folder path this document sits in; '' at the root. */
  path: string
  initiative: Initiative
}

export interface TreeFolder {
  kind: 'folder'
  /** This folder's own name; '' for the root. */
  name: string
  /** Full path from the root, e.g. `product/billing`. '' for the root. */
  path: string
  children: TreeNode[]
  /** Documents anywhere beneath this folder — what a collapsed row shows. */
  count: number
}

export type TreeNode = TreeFolder | TreeDoc

/**
 * Clean a user- or agent-supplied path into canonical segments.
 *
 * Traversal segments are dropped rather than rejected: a path is a display
 * grouping, not a filesystem location, so `../` means nothing here and silently
 * meaning nothing is better than refusing an initiative someone can then never
 * see. Depth beyond MAX_DEPTH is folded into the last segment so no document
 * becomes unreachable by having been filed too deep.
 */
export function normalizePath(raw: unknown): string {
  if (typeof raw !== 'string') return ''
  const parts = raw
    .split('/')
    .map((s) => s.trim())
    .filter((s) => s !== '' && s !== '.' && s !== '..')
    .map((s) => (s.length > MAX_SEGMENT ? s.slice(0, MAX_SEGMENT).trim() : s))
  if (parts.length <= MAX_DEPTH) return parts.join('/')
  const head = parts.slice(0, MAX_DEPTH - 1)
  head.push(parts.slice(MAX_DEPTH - 1).join(' '))
  return head.join('/')
}

/** The folder an initiative is filed in, '' when it has never been filed. */
export function pathOf(i: Initiative): string {
  const meta = i.metadata
  if (!meta || typeof meta !== 'object') return ''
  return normalizePath((meta as Record<string, unknown>).path)
}

/** Every ancestor path of a path, root first: `a/b/c` → ['a', 'a/b', 'a/b/c']. */
export function ancestors(path: string): string[] {
  if (!path) return []
  const out: string[] = []
  let acc = ''
  for (const seg of path.split('/')) {
    acc = acc ? `${acc}/${seg}` : seg
    out.push(acc)
  }
  return out
}

/** Folders first, then documents; each alphabetical, case-insensitively. */
function order(a: TreeNode, b: TreeNode): number {
  if (a.kind !== b.kind) return a.kind === 'folder' ? -1 : 1
  const an = a.kind === 'folder' ? a.name : a.initiative.title
  const bn = b.kind === 'folder' ? b.name : b.initiative.title
  return an.localeCompare(bn, undefined, { sensitivity: 'base' })
}

/**
 * Build the tree. Folders are created by the documents that reference them, so an
 * empty folder cannot exist — which is the right trade for a grouping that has no
 * row of its own: nothing can be left behind when the last document moves out.
 */
export function buildTree(items: Initiative[]): TreeFolder {
  const root: TreeFolder = { kind: 'folder', name: '', path: '', children: [], count: 0 }
  const folders = new Map<string, TreeFolder>([['', root]])

  const folderAt = (path: string): TreeFolder => {
    const existing = folders.get(path)
    if (existing) return existing
    const cut = path.lastIndexOf('/')
    const parent = folderAt(cut === -1 ? '' : path.slice(0, cut))
    const made: TreeFolder = {
      kind: 'folder',
      name: cut === -1 ? path : path.slice(cut + 1),
      path,
      children: [],
      count: 0,
    }
    folders.set(path, made)
    parent.children.push(made)
    return made
  }

  for (const initiative of items) {
    const path = pathOf(initiative)
    folderAt(path).children.push({ kind: 'doc', path, initiative })
    // Every ancestor counts it, so a collapsed folder reports what is under it
    // rather than only what is directly inside it.
    root.count += 1
    for (const a of ancestors(path)) folderAt(a).count += 1
  }

  const sort = (f: TreeFolder): void => {
    f.children.sort(order)
    for (const c of f.children) if (c.kind === 'folder') sort(c)
  }
  sort(root)
  return root
}

/** Every folder path in the tree, for a move target list. */
export function folderPaths(root: TreeFolder): string[] {
  const out: string[] = []
  const walk = (f: TreeFolder): void => {
    for (const c of f.children) {
      if (c.kind !== 'folder') continue
      out.push(c.path)
      walk(c)
    }
  }
  walk(root)
  return out.sort()
}

/**
 * Filter the tree to documents matching a predicate, keeping the folders on the
 * way to a survivor and dropping the ones left empty — so typing in the filter
 * box collapses the tree onto the hits rather than leaving hollow rows.
 */
export function pruneTree(root: TreeFolder, keep: (i: Initiative) => boolean): TreeFolder {
  const walk = (f: TreeFolder): TreeFolder => {
    const children: TreeNode[] = []
    let count = 0
    for (const c of f.children) {
      if (c.kind === 'doc') {
        if (keep(c.initiative)) {
          children.push(c)
          count += 1
        }
        continue
      }
      const sub = walk(c)
      if (sub.count > 0) {
        children.push(sub)
        count += sub.count
      }
    }
    return { ...f, children, count }
  }
  return walk(root)
}
