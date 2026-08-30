// What ⌘K offers, and how "go to node…" finds a node.
//
// The rail is gone: the canvas IS the page, so this is the only place that
// answers "what can I do here" and the only way to reach a node that is not on
// screen. Both halves are pure so they can be tested — jsdom has no layout
// engine, so a component test could prove nothing about either.
//
// Two rules the palette lives by:
//
//   * A command that does not apply is ABSENT, never disabled. A list of
//     things it will refuse is noise, and on a read-only token it would be
//     most of the list.
//   * Scope is the selected node, else the map. Node commands come first
//     because they are what somebody who selected a node came for.
import { MAX_ATTACHMENTS } from './mindmap-doc'

export const NODE_COMMANDS = [
  'node.child',
  'node.sibling',
  'node.rename',
  'node.notes',
  'node.relate',
  'node.attach',
  'node.promoteEpic',
  'node.promoteInitiative',
  'node.collapse',
  'node.expand',
  'node.delete',
] as const

export const MAP_COMMANDS = [
  'map.goto',
  'map.fit',
  'map.tidy',
  'map.rename',
  'map.project',
  'map.delete',
] as const

export type CommandId = (typeof NODE_COMMANDS)[number] | (typeof MAP_COMMANDS)[number]

/** The selected node, reduced to the facts a command list depends on. */
export interface CommandNode {
  id: string
  title: string
  /** A branch that already graduated cannot graduate again. */
  promoted: boolean
  attachments: number
  /** Has at least one child in the WHOLE tree, folded or not. */
  hasChildren: boolean
  /** Folded by THIS viewer — fold is per-viewer and never in the document. */
  collapsed: boolean
}

export interface CommandContext {
  /** The socket ticket's write bit: can this browser change the document. */
  canWrite: boolean
  /**
   * Whether the page's own token may change the MAP — rename, delete, promote.
   * Those go over REST rather than the socket, so they are a separate question
   * from `canWrite`.
   */
  canManageMap: boolean
  node: CommandNode | null
  nodeCount: number
  /** How many projects this token can reach. One is nothing to switch between. */
  projectCount: number
}

/** Every command that APPLIES right now, node scope first. */
export function commandsFor(ctx: CommandContext): CommandId[] {
  const out: CommandId[] = []
  const n = ctx.node
  if (n) {
    if (ctx.canWrite) {
      out.push('node.child', 'node.sibling', 'node.rename', 'node.notes')
      if (ctx.nodeCount >= 2) out.push('node.relate')
      if (n.attachments < MAX_ATTACHMENTS) out.push('node.attach')
    }
    if (ctx.canManageMap && !n.promoted) out.push('node.promoteEpic', 'node.promoteInitiative')
    if (n.collapsed) out.push('node.expand')
    else if (n.hasChildren) out.push('node.collapse')
    if (ctx.canWrite) out.push('node.delete')
  }
  if (ctx.nodeCount > 0) out.push('map.goto')
  out.push('map.fit')
  if (ctx.canWrite && ctx.nodeCount > 0) out.push('map.tidy')
  if (ctx.canManageMap) out.push('map.rename')
  if (ctx.projectCount > 1) out.push('map.project')
  if (ctx.canManageMap) out.push('map.delete')
  return out
}

/**
 * How well `text` answers `query`, lower being better, or null for no match.
 *
 * Ranked in CLASSES rather than by one blended number, so the order is
 * explainable: an exact title beats a prefix beats a word start beats a
 * substring beats a subsequence. Within a class the tie-breaks are small
 * fractions — where the match starts, and for a subsequence how far it is
 * spread — so they can never promote a worse class.
 *
 * The subsequence class is what makes this fuzzy rather than a filter: "brd"
 * finds "Broaden the redesign", which is how somebody types at conversation
 * speed.
 */
export function matchScore(text: string, query: string): number | null {
  const t = text.toLowerCase()
  const q = query.trim().toLowerCase()
  if (!q) return 0
  if (t === q) return 0
  if (t.startsWith(q)) return 1 + Math.min(t.length, 999) / 10000

  const at = t.indexOf(q)
  if (at >= 0) {
    // A match that starts a word reads as intentional; one mid-word is weaker.
    const before = t[at - 1] ?? ' '
    const boundary = /[\s\-_/·:,.]/.test(before)
    return (boundary ? 2 : 3) + Math.min(at, 999) / 10000
  }

  // Subsequence: every character of the query, in order, anywhere.
  let cursor = -1
  let first = -1
  for (const ch of q) {
    const found = t.indexOf(ch, cursor + 1)
    if (found === -1) return null
    if (first === -1) first = found
    cursor = found
  }
  const spread = cursor - first + 1 - q.length
  return 4 + Math.min(spread, 999) / 1000 + Math.min(first, 999) / 100000
}

/**
 * Rank `items` against a query, keeping the given order within a class.
 *
 * An empty query keeps the caller's order — for "go to node…" that is the tree
 * order, which is the useful default: the first thing offered is the top of the
 * map, not whatever sorted first alphabetically.
 */
export function fuzzyRank<T>(
  items: readonly T[],
  textOf: (item: T) => string,
  query: string,
  max: number,
): T[] {
  const q = query.trim()
  if (!q) return items.slice(0, max)
  const scored: { item: T; score: number; index: number }[] = []
  items.forEach((item, index) => {
    const score = matchScore(textOf(item), q)
    if (score !== null) scored.push({ item, score, index })
  })
  scored.sort((a, b) => a.score - b.score || a.index - b.index)
  return scored.slice(0, max).map((s) => s.item)
}

/**
 * Whether a keystroke landed in a text field.
 *
 * ⌘K must not open over somebody's typing — it may only CLOSE from there — and
 * the check is a pure function of the element's shape so it can be tested
 * without a DOM.
 */
export function isTextEntry(
  element: { tagName?: string; isContentEditable?: boolean } | null | undefined,
): boolean {
  if (!element) return false
  if (element.isContentEditable) return true
  const tag = (element.tagName ?? '').toUpperCase()
  return tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT'
}
