// The document view of an initiative: three panes derived from the entry log.
//
// An initiative accumulates entries. Reading them newest-first tells you what
// was WORKED ON; it does not tell you what is now UNDERSTOOD. So two reserved
// entry kinds turn the same log into a document:
//
//   `view`   one pane's prose.  meta: { pane, cites: [entryId, …] }
//   `thread` a margin note.     meta: { pane, para, state? }
//
// Everything else — transcript, sample-data, code-research, research, note — is
// EVIDENCE: citable from a pane, and listed in the lineage footer.
//
// Nothing here is stored. The document is reduced from the entries on every
// read, exactly like `rollup` is on the server, which is what keeps it from
// drifting from the thing it describes. There is no schema change behind any of
// this: `kind` is already a free-form slug and `meta` is already a free-form
// JSON object on every entry.
import type { Entry } from './initiatives'

export const PANES = ['business', 'technical', 'verification'] as const
export type Pane = (typeof PANES)[number]

/** Entry kinds this module reserves. Everything else is evidence. */
export const VIEW_KIND = 'view'
export const THREAD_KIND = 'thread'

export type ThreadState = 'open' | 'running' | 'resolved'

/** One run of a paragraph: either literal text, or a citation mark. */
export type Run = { text: string } | { cite: number; entry: Entry }

export interface Paragraph {
  runs: Run[]
  /** No citation anywhere in the paragraph — an assertion nobody sourced. */
  uncited: boolean
}

export interface Thread {
  entry: Entry
  /** Index of the paragraph this note sits beside. Always in range. */
  para: number
  state: ThreadState
}

export interface PaneDoc {
  pane: Pane
  /** The `view` entry this pane was reduced from, or null when unwritten. */
  entry: Entry | null
  paragraphs: Paragraph[]
  threads: Thread[]
}

export interface Doc {
  panes: Record<Pane, PaneDoc>
  /** Cited evidence in first-citation order; a source's number is its index + 1. */
  sources: Entry[]
  /** False when no pane has been written — the page then stays on the log. */
  hasDocument: boolean
}

function isPane(v: unknown): v is Pane {
  return typeof v === 'string' && (PANES as readonly string[]).includes(v)
}

function meta(entry: Entry): Record<string, unknown> {
  const m = entry.meta
  return m && typeof m === 'object' && !Array.isArray(m) ? (m as Record<string, unknown>) : {}
}

function paneOf(entry: Entry): Pane | null {
  const p = meta(entry).pane
  return isPane(p) ? p : null
}

function citesOf(entry: Entry): string[] {
  const c = meta(entry).cites
  return Array.isArray(c) ? c.filter((x): x is string => typeof x === 'string') : []
}

function stateOf(entry: Entry): ThreadState {
  const s = meta(entry).state
  return s === 'running' || s === 'resolved' ? s : 'open'
}

/**
 * Newest wins. Entries arrive newest-first from the API, but that is the
 * caller's ordering rather than a guarantee, so compare `created_at` and fall
 * back to id — two entries written in the same millisecond must still resolve
 * to one deterministic winner rather than whichever the sort happened to keep.
 */
function newer(a: Entry, b: Entry): Entry {
  if (a.created_at !== b.created_at) return a.created_at > b.created_at ? a : b
  return a.id > b.id ? a : b
}

/** `[3]` — a citation mark. The number indexes THAT pane's `cites` array, 1-based. */
const MARK = /\[(\d{1,3})\]/g

/**
 * Split prose into paragraphs on blank lines, then each paragraph into text runs
 * and citation marks.
 *
 * A mark whose index falls outside the pane's `cites` is left as literal text:
 * a stale citation must read as the broken thing it is, not silently vanish.
 * `resolve` maps an entry id to its global source number.
 */
function parseProse(
  text: string,
  cites: string[],
  byId: Map<string, Entry>,
  numberOf: Map<string, number>,
): Paragraph[] {
  return text
    .split(/\n\s*\n/)
    .map((p) => p.trim())
    .filter(Boolean)
    .map((block) => {
      const runs: Run[] = []
      let cited = false
      let last = 0
      // `matchAll` needs the global flag, and a shared regex would carry
      // `lastIndex` between paragraphs — build the matches per block.
      for (const m of block.matchAll(MARK)) {
        const idx = Number(m[1]) - 1
        const id = cites[idx]
        const entry = id ? byId.get(id) : undefined
        const n = id ? numberOf.get(id) : undefined
        if (!entry || n === undefined) continue // stale — leave the literal text
        const at = m.index ?? 0
        if (at > last) runs.push({ text: block.slice(last, at) })
        runs.push({ cite: n, entry })
        cited = true
        last = at + m[0].length
      }
      if (last < block.length) runs.push({ text: block.slice(last) })
      return { runs, uncited: !cited }
    })
}

/**
 * Reduce an entry log to the document. Pure: the same entries always produce the
 * same document, which is what makes it testable without a server.
 */
export function buildDoc(entries: Entry[]): Doc {
  const byId = new Map(entries.map((e) => [e.id, e]))

  // Latest `view` per pane.
  const views = new Map<Pane, Entry>()
  for (const e of entries) {
    if (e.kind !== VIEW_KIND) continue
    const p = paneOf(e)
    if (!p) continue
    const cur = views.get(p)
    views.set(p, cur ? newer(cur, e) : e)
  }

  // Global citation numbering: panes in fixed order, each pane's cites in the
  // order the author listed them. An author only ever needs LOCAL indices — the
  // numbering a reader sees is assigned here, so two panes citing the same
  // entry show the same number.
  const numberOf = new Map<string, number>()
  const sources: Entry[] = []
  for (const p of PANES) {
    const v = views.get(p)
    if (!v) continue
    for (const id of citesOf(v)) {
      if (numberOf.has(id)) continue
      const entry = byId.get(id)
      if (!entry) continue // cites something outside this page of entries
      sources.push(entry)
      numberOf.set(id, sources.length)
    }
  }

  const panes = {} as Record<Pane, PaneDoc>
  for (const p of PANES) {
    const v = views.get(p) ?? null
    const paragraphs = v ? parseProse(v.text ?? '', citesOf(v), byId, numberOf) : []
    const threads: Thread[] = entries
      .filter((e) => e.kind === THREAD_KIND && paneOf(e) === p)
      .map((e) => {
        const raw = meta(e).para
        const n = typeof raw === 'number' && Number.isInteger(raw) ? raw : 0
        // Clamp rather than drop: a note that outlived the paragraph it was
        // written against is still someone's unanswered question.
        const para = paragraphs.length === 0 ? 0 : Math.min(Math.max(n, 0), paragraphs.length - 1)
        return { entry: e, para, state: stateOf(e) }
      })
    panes[p] = { pane: p, entry: v, paragraphs, threads }
  }

  return { panes, sources, hasDocument: views.size > 0 }
}

/** Threads for one paragraph, oldest first — the order the argument happened in. */
export function threadsFor(doc: PaneDoc, para: number): Thread[] {
  return doc.threads
    .filter((t) => t.para === para)
    .sort((a, b) => a.entry.created_at.localeCompare(b.entry.created_at))
}
