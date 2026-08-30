// What a node SAYS, as opposed to where it sits.
//
// `mindmap-layout.ts` answers where a thought is drawn and `mindmap-doc.ts`
// answers what the document holds; this module answers the third question the
// canvas keeps asking — what one line of this node is worth showing when there is
// only room for one line. Four separate readings, all of them pure so they can be
// tested without a canvas that jsdom cannot lay out anyway:
//
//   * the first sentence of somebody's notes, so a map of thirty nodes reads as
//     thirty thoughts rather than thirty labels;
//   * what a folded branch is holding, so folding reads as SUMMARISING rather
//     than hiding;
//   * how much anybody has actually confirmed about a node, which is the trust
//     lens;
//   * which node a question is about, and what detaching an edge would do.
//
// None of it is stored. Every reading is derived on read from fields the document
// already carries, which is what lets an agent grow a map through the API and a
// person see all of this without either side writing a presentation field.
import { descendantsOf, type MapNode, type Relationship } from './mindmap-doc'

// ---- one line of substance -------------------------------------------------

/**
 * Words that end in a full stop without ending a sentence.
 *
 * The list is short and stays short: it exists so "Ship it by Q3, e.g. after the
 * migration." does not get cut to "Ship it by Q3, e.g." — not so this module can
 * pass a linguistics exam. Anything unlisted simply ends the sentence, which is
 * the failure that costs least: a slightly short line rather than a wrong one.
 */
const ABBREVIATIONS = new Set([
  'e.g',
  'i.e',
  'eg',
  'ie',
  'etc',
  'vs',
  'cf',
  'approx',
  'fig',
  'no',
  'al',
  'ca',
  'dr',
  'mr',
  'mrs',
  'ms',
  'st',
  'jr',
  'sr',
  'prof',
])

/** Trim to `max` characters on a word boundary, with an ellipsis when it bit. */
export function clampText(text: string, max: number): string {
  if (text.length <= max) return text
  const cut = text.slice(0, max)
  const space = cut.lastIndexOf(' ')
  return `${(space > max * 0.6 ? cut.slice(0, space) : cut).trimEnd()}…`
}

/** Does the text before a full stop end in something that is not a sentence end? */
function abbreviated(head: string): boolean {
  const word = /([A-Za-z.]+)$/.exec(head)?.[1] ?? ''
  if (!word) return false
  // A lone initial — "J." in "J. Random" — is never a sentence end either.
  if (word.length === 1) return true
  return ABBREVIATIONS.has(word.toLowerCase())
}

/**
 * The first sentence of some prose, whitespace flattened and length capped.
 *
 * Three cases carry the whole design. Notes with no terminator at all are the
 * COMMON case — a note is usually one unpunctuated fragment — so the whole thing
 * comes back rather than nothing. An abbreviation mid-sentence does not end it.
 * And empty notes give an empty string, never a stray "…", because the caller
 * uses emptiness to decide whether to draw the line at all.
 */
export function firstSentence(notes: string, max = 160): string {
  const text = notes.replace(/\s+/g, ' ').trim()
  if (!text) return ''
  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i]
    if (ch !== '.' && ch !== '!' && ch !== '?') continue
    // "3.5" and "e.g." both keep going: a terminator is only a terminator when
    // the sentence actually stops after it.
    const next = text[i + 1]
    if (next !== undefined && next !== ' ') continue
    if (ch === '.' && abbreviated(text.slice(0, i))) continue
    return clampText(text.slice(0, i + 1), max)
  }
  return clampText(text, max)
}

// ---- what a fold is holding ------------------------------------------------

export interface FoldSummary {
  /** Every thought under the folded node, not just the titles that fitted. */
  count: number
  /** Those titles, in tree order, joined and clamped. */
  text: string
}

/**
 * What disappeared when this viewer folded a branch.
 *
 * Folding used to leave a number and nothing else, which is hiding. A count plus
 * the titles is summarising: the reader can decide whether the branch is worth
 * unfolding without unfolding it. The count is of EVERYTHING beneath, while the
 * text is whatever fitted — so a summary that was clamped still says how much it
 * is standing in for.
 */
export function foldSummary(
  nodes: readonly MapNode[],
  id: string,
  max = 120,
): FoldSummary | null {
  const ids = descendantsOf(nodes, id)
  if (ids.length === 0) return null
  const byId = new Map(nodes.map((n) => [n.id, n]))
  const titles = ids
    .map((child) => byId.get(child)?.title.trim() ?? '')
    .filter((title) => title.length > 0)
  return { count: ids.length, text: clampText(titles.join(' · '), max) }
}

// ---- the trust lens --------------------------------------------------------

/**
 * How much anybody has actually confirmed about a node.
 *
 * `origin` and `reviewed` were already stored and already written by both the API
 * and the canvas; nothing here is new state. What is new is asking them TOGETHER,
 * because on a map an agent has been growing the useful question is not "who
 * wrote this" but "what in here has nobody looked at".
 */
export type Trust = 'confirmed' | 'machine' | 'unverified'

export function trustOf(node: { origin: MapNode['origin']; reviewed: boolean }): Trust {
  // `reviewed` decides first, whoever wrote it. The flag means a person has
  // looked at this — and an agent's suggestion somebody has read and kept is
  // exactly as confirmed as a thought they typed themselves. Reading `origin`
  // first would leave a node marked as unchecked *because* a machine wrote it,
  // no matter how many people had since agreed with it, which is the one answer
  // this lens must not give.
  if (node.reviewed) return 'confirmed'
  if (node.origin === 'agent') return 'machine'
  return 'unverified'
}

// ---- questions -------------------------------------------------------------

/**
 * What a question node is asking ABOUT, or null when it asks about nothing.
 *
 * A question hangs off an ordinary relationship rather than a field of its own,
 * so nothing in the store had to learn about questions and an agent can pose one
 * with the two calls it already has. Relationships arrive sorted by id, so the
 * first one touching the question is a stable answer rather than whichever the
 * map happened to yield first.
 */
export function questionTarget(
  relationships: readonly Relationship[],
  questionId: string,
): string | null {
  for (const r of relationships) {
    if (r.from === questionId) return r.to
    if (r.to === questionId) return r.from
  }
  return null
}

/**
 * Somebody's answer, added to whatever notes were already there.
 *
 * Appended, never replacing: the answer is a person's own words and the notes may
 * be somebody else's. A blank answer changes nothing, which is what stops an
 * empty box from clearing a paragraph.
 */
export function appendAnswer(notes: string, answer: string): string {
  const text = answer.trim()
  if (!text) return notes
  const existing = notes.trim()
  return existing ? `${existing}\n\n${text}` : text
}

// ---- cutting an edge -------------------------------------------------------

export interface CutTarget {
  child: { id: string; title: string }
  /** What it currently hangs off, so the question can name both ends. */
  parentTitle: string
}

/**
 * What clicking a hierarchy edge would detach, or null when there is nothing to
 * detach.
 *
 * An edge is drawn per CHILD — a node has exactly one parent — so a click
 * resolves to the child and the cut is "this thought stops hanging off that one".
 * A first-ring node's line goes to the map itself rather than to a node, and
 * cutting it would mean nothing; that is the null.
 */
export function cutTarget(nodes: readonly MapNode[], childId: string): CutTarget | null {
  const child = nodes.find((n) => n.id === childId)
  if (!child || child.parent === null) return null
  const parent = nodes.find((n) => n.id === child.parent)
  if (!parent) return null
  return { child: { id: child.id, title: child.title }, parentTitle: parent.title }
}
