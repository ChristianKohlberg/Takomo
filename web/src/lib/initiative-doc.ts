// The document view of an initiative: three panes derived from the entry log.
//
// An initiative accumulates entries. Reading them newest-first tells you what
// was WORKED ON; it does not tell you what is now UNDERSTOOD. So a handful of
// reserved entry kinds turn the same log into a document:
//
//   `view`     one pane's prose.  meta: { pane, cites: [entryId, …] }
//              …with `proposed: true` it is an AMENDMENT awaiting a decision
//              rather than the live pane.
//   `thread`   a margin note.     meta: { pane, para, state?, ticket?, supersedes? }
//   `decision` a human's verdict. meta: { accepts | rejects: <proposalId> }
//
// Any entry may also carry `meta.origin: true`, which marks it as how the idea
// ARRIVED — the customer's own words, quoted at the top of the document.
//
// Everything else — transcript, sample-data, code-research, research, note — is
// EVIDENCE: citable from a pane, and listed in the lineage footer.
//
// Nothing here is stored. The document is reduced from the entries on every
// read, exactly like `rollup` is on the server, which is what keeps it from
// drifting from the thing it describes. There is no schema change behind any of
// it: `kind` is already a free-form slug and `meta` a free-form JSON object.
//
// EVERY mutation is an append. A pane is revised by appending a new `view`; a
// thread changes state by appending a `thread` that supersedes it; a proposal is
// decided by appending a `decision`. Nothing is ever edited or deleted, so the
// argument that produced the current text is always still readable.
import type { Entry } from './initiatives'

export const PANES = ['business', 'technical', 'verification'] as const
export type Pane = (typeof PANES)[number]

export const VIEW_KIND = 'view'
export const THREAD_KIND = 'thread'
export const DECISION_KIND = 'decision'

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
  para: number
  state: ThreadState
  /** The ticket this note was dispatched as, when it has been. */
  ticket: string | null
}

export type DiffRow =
  | { kind: 'same' | 'added' | 'removed'; text: string }
  | { kind: 'changed'; text: string; was: string }

export interface Amendment {
  /** The proposed `view` entry, still undecided. */
  entry: Entry
  paragraphs: Paragraph[]
  /** Paragraph-level comparison against the live pane. */
  diff: DiffRow[]
}

export interface PaneDoc {
  pane: Pane
  /** The live `view` entry this pane was reduced from, or null when unwritten. */
  entry: Entry | null
  paragraphs: Paragraph[]
  threads: Thread[]
  /** An amendment awaiting accept or reject, newest first. */
  pending: Amendment | null
}

export interface Doc {
  panes: Record<Pane, PaneDoc>
  /** How the idea arrived, oldest first — the customer's own words. */
  origins: Entry[]
  /** Cited evidence in first-citation order; a source's number is its index + 1. */
  sources: Entry[]
  hasDocument: boolean
}

function isPane(v: unknown): v is Pane {
  return typeof v === 'string' && (PANES as readonly string[]).includes(v)
}

function meta(entry: Entry): Record<string, unknown> {
  const m = entry.meta
  return m && typeof m === 'object' && !Array.isArray(m) ? (m as Record<string, unknown>) : {}
}

function str(v: unknown): string | null {
  return typeof v === 'string' && v ? v : null
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

function isProposal(entry: Entry): boolean {
  return meta(entry).proposed === true
}

/**
 * Newest wins, with id as the tie-break: two entries written in the same
 * millisecond must still resolve to one deterministic winner rather than
 * whichever the sort happened to keep.
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
 * A mark whose index falls outside the pane's `cites` is left as literal text: a
 * stale citation must read as the broken thing it is, not silently vanish.
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

/** A paragraph flattened back to plain text, for comparison and for a diff row. */
export function paraText(p: Paragraph): string {
  return p.runs
    .map((r) => ('text' in r ? r.text : `[${r.cite}]`))
    .join('')
    .trim()
}

/**
 * Paragraph-level comparison, positionally.
 *
 * Deliberately NOT a word diff: an amendment here rewrites whole paragraphs of
 * argument, and a word diff of two paragraphs of prose is noise a reviewer has
 * to decode. What a reviewer needs is "these two paragraphs changed, this one is
 * new" — at which point they read both versions.
 */
function diffParagraphs(live: Paragraph[], next: Paragraph[]): DiffRow[] {
  const rows: DiffRow[] = []
  const n = Math.max(live.length, next.length)
  for (let i = 0; i < n; i += 1) {
    const a = live[i]
    const b = next[i]
    if (a && b) {
      const at = paraText(a)
      const bt = paraText(b)
      rows.push(at === bt ? { kind: 'same', text: bt } : { kind: 'changed', text: bt, was: at })
    } else if (b) {
      rows.push({ kind: 'added', text: paraText(b) })
    } else if (a) {
      rows.push({ kind: 'removed', text: paraText(a) })
    }
  }
  return rows
}

/**
 * Reduce an entry log to the document. Pure: the same entries always produce the
 * same document, which is what makes it testable without a server.
 */
export function buildDoc(entries: Entry[]): Doc {
  const byId = new Map(entries.map((e) => [e.id, e]))

  // Decisions first — a proposal that has been accepted or rejected is no longer
  // pending, and must not reappear as something to review a second time.
  const decided = new Set<string>()
  for (const e of entries) {
    if (e.kind !== DECISION_KIND) continue
    const m = meta(e)
    const id = str(m.accepts) ?? str(m.rejects)
    if (id) decided.add(id)
  }

  // Latest live `view` per pane, and separately the newest undecided proposal.
  const views = new Map<Pane, Entry>()
  const proposals = new Map<Pane, Entry>()
  for (const e of entries) {
    if (e.kind !== VIEW_KIND) continue
    const p = paneOf(e)
    if (!p) continue
    if (isProposal(e)) {
      if (decided.has(e.id)) continue
      const cur = proposals.get(p)
      proposals.set(p, cur ? newer(cur, e) : e)
    } else {
      const cur = views.get(p)
      views.set(p, cur ? newer(cur, e) : e)
    }
  }

  // Global citation numbering: panes in fixed order, each pane's cites in the
  // order the author listed them. An author only ever needs LOCAL indices — the
  // numbering a reader sees is assigned here, so two panes citing the same entry
  // show the same number. Proposals are numbered too, so a pending amendment's
  // marks resolve before it has been accepted.
  const numberOf = new Map<string, number>()
  const sources: Entry[] = []
  const number = (id: string) => {
    if (numberOf.has(id)) return
    const entry = byId.get(id)
    if (!entry) return // cites something outside this page of entries
    sources.push(entry)
    numberOf.set(id, sources.length)
  }
  for (const p of PANES) {
    const v = views.get(p)
    if (v) citesOf(v).forEach(number)
  }
  for (const p of PANES) {
    const v = proposals.get(p)
    if (v) citesOf(v).forEach(number)
  }

  // A thread that supersedes another replaces it: same conversation, new state.
  const superseded = new Set<string>()
  for (const e of entries) {
    if (e.kind !== THREAD_KIND) continue
    const s = str(meta(e).supersedes)
    if (s) superseded.add(s)
  }

  const panes = {} as Record<Pane, PaneDoc>
  for (const p of PANES) {
    const v = views.get(p) ?? null
    const paragraphs = v ? parseProse(v.text ?? '', citesOf(v), byId, numberOf) : []

    const threads: Thread[] = entries
      .filter((e) => e.kind === THREAD_KIND && paneOf(e) === p && !superseded.has(e.id))
      .map((e) => {
        const raw = meta(e).para
        const n = typeof raw === 'number' && Number.isInteger(raw) ? raw : 0
        // Clamp rather than drop: a note that outlived the paragraph it was
        // written against is still someone's unanswered question.
        const para = paragraphs.length === 0 ? 0 : Math.min(Math.max(n, 0), paragraphs.length - 1)
        return { entry: e, para, state: stateOf(e), ticket: str(meta(e).ticket) }
      })

    const prop = proposals.get(p) ?? null
    const pending: Amendment | null = prop
      ? (() => {
          const next = parseProse(prop.text ?? '', citesOf(prop), byId, numberOf)
          return { entry: prop, paragraphs: next, diff: diffParagraphs(paragraphs, next) }
        })()
      : null

    panes[p] = { pane: p, entry: v, paragraphs, threads, pending }
  }

  // How it came in. Oldest FIRST here, against the newest-first rule everywhere
  // else: this is the beginning of the story and reads in the order it happened.
  const origins = entries
    .filter((e) => meta(e).origin === true)
    .sort((a, b) => (a.origin_at ?? a.created_at).localeCompare(b.origin_at ?? b.created_at))

  return { panes, origins, sources, hasDocument: views.size > 0 }
}

/** Threads for one paragraph, oldest first — the order the argument happened in. */
export function threadsFor(doc: PaneDoc, para: number): Thread[] {
  return doc.threads
    .filter((t) => t.para === para)
    .sort((a, b) => a.entry.created_at.localeCompare(b.entry.created_at))
}
