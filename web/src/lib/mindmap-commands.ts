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
export const NODE_COMMANDS = [
  'node.child',
  'node.sibling',
  'node.open',
  'node.rename',
  'node.relate',
  'node.attach',
  'node.ask',
  'node.promoteEpic',
  'node.promoteInitiative',
  'node.collapse',
  'node.expand',
  'node.delete',
] as const

export const MAP_COMMANDS = [
  'map.plan',
  'map.tests',
  'map.goto',
  'map.fit',
  'map.trust',
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
  /** How many pointers hang off it. The badge draws this; the palette does not
   *  gate on it, because the manager is also how one is REMOVED. */
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
    // Opening a thought is a READ — it is where the notes, the attachments and
    // the lines to other branches are, now that selecting a node no longer
    // throws a panel over the map — so it survives a read-only token.
    out.push('node.open')
    if (ctx.canWrite) {
      out.push('node.child', 'node.sibling', 'node.rename')
      if (ctx.nodeCount >= 2) out.push('node.relate')
      // Offered at the cap too, now that it opens the manager rather than an
      // inline add row: a node with twenty attachments is exactly the one whose
      // list somebody needs to get INTO.
      out.push('node.attach')
      // Asking about a node is a WRITE — it puts a question node on a map other
      // people are reading — so it lives with the other write verbs and is
      // absent on a read-only token like all of them.
      out.push('node.ask')
    }
    if (ctx.canManageMap && !n.promoted) out.push('node.promoteEpic', 'node.promoteInitiative')
    if (n.collapsed) out.push('node.expand')
    else if (n.hasChildren) out.push('node.collapse')
    if (ctx.canWrite) out.push('node.delete')
  }
  // The plan is this map written out — the same tree, read as reading order —
  // so it is the one map command that is always offered: it survives a
  // read-only token (reading the plan is a read) and an empty map (which is a
  // plan with no sections rather than no plan). With a node selected it lands on
  // that section; `spec/one-model-two-views.md`.
  out.push('map.plan')
  // The third view of the same tree: what has to pass before this part is done.
  // A read like the plan, and offered on the same terms — with a node selected
  // it lands filtered to that section.
  out.push('map.tests')
  if (ctx.nodeCount > 0) out.push('map.goto')
  out.push('map.fit')
  // The trust lens is per-viewer and reads nothing but fields already on screen,
  // so it survives a read-only token exactly as fold does.
  if (ctx.nodeCount > 0) out.push('map.trust')
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

/**
 * The verbs the floating pill offers over the selected node.
 *
 * Deliberately at most FOUR, and deliberately a SUBSET of the command ids above
 * rather than a vocabulary of its own — a pill that grew its own actions would
 * be the side panel coming back in a rounder shape, and ⌘K already carries the
 * long tail.
 *
 * Which four is decided by what the other affordances already cover. Adding a
 * child has its own `+` beside the node, attachments have the badge on it, and
 * removing it is on the right-click menu, so none of those is here. What is left
 * is what somebody reaches for while THINKING rather than while arranging:
 * opening the thought, renaming it, folding the branch they have finished
 * reading, and drawing a line to something else.
 *
 * Opening it is FIRST and survives a read-only token, because selecting a node
 * no longer expands it into a reading panel — this is the way in. Fold survives
 * for its own reason: it is per-viewer and never touches the document.
 */
export function pillVerbsFor(ctx: CommandContext): CommandId[] {
  const n = ctx.node
  if (!n) return []
  const out: CommandId[] = ['node.open']
  if (ctx.canWrite) out.push('node.rename')
  if (n.collapsed) out.push('node.expand')
  else if (n.hasChildren) out.push('node.collapse')
  if (ctx.canWrite && ctx.nodeCount >= 2) out.push('node.relate')
  return out
}

/**
 * What right-clicking a node offers.
 *
 * The menu is where the verbs that are OCCASIONAL live — the ones that would
 * make the pill a menu. Removing the node is the reason it exists at all, and it
 * is last and separated, because a menu whose destructive item sits between two
 * ordinary ones is a menu that eventually deletes a branch by accident.
 */
export function menuVerbsFor(ctx: CommandContext): CommandId[] {
  const n = ctx.node
  if (!n) return []
  // First, and unconditional: right-click is the one gesture that reaches a node
  // without selecting it, so it has to be able to open one.
  const out: CommandId[] = ['node.open']
  if (ctx.canWrite) out.push('node.child', 'node.sibling', 'node.rename', 'node.attach')
  if (n.collapsed) out.push('node.expand')
  else if (n.hasChildren) out.push('node.collapse')
  if (ctx.canManageMap && !n.promoted) out.push('node.promoteEpic', 'node.promoteInitiative')
  if (ctx.canWrite) out.push('node.delete')
  return out
}
