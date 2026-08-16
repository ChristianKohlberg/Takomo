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
import { anchorOf, resolveAnchor, type Anchor, type Placed } from './initiative-anchor'
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
  /** The words this note was written against, when it carries a range anchor. */
  anchor: Anchor | null
  /**
   * Where the anchor sits in the CURRENT prose. Null means one of two things,
   * distinguished by `anchor`: a legacy paragraph-only note (no anchor), or a
   * note whose words are gone (`orphaned`).
   */
  placed: Placed | null
  /** The prose this note was about no longer exists. Shown, never hidden. */
  orphaned: boolean
}

export type DiffRow =
  | { kind: 'same' | 'added' | 'removed'; text: string }
  | { kind: 'changed'; text: string; was: string }

/**
 * A proposed change awaiting a decision.
 *
 * Two scopes, because two different things want to propose. An agent that has
 * rethought a whole argument replaces the pane; a reader who highlighted one
 * sentence proposes new words for exactly that range. The second is what makes
 * suggesting feel like suggesting rather than like submitting a rewrite, and it
 * is why several can be pending at once — a pane-scoped proposal is a single
 * take-it-or-leave-it, and stacking those was never useful.
 */
export interface Amendment {
  /** The proposed `view` entry, still undecided. */
  entry: Entry
  scope: 'pane' | 'range'
  /** Range scope only: the words being replaced, and where they now sit. */
  anchor: Anchor | null
  placed: Placed | null
  /** Range scope only: the text offered in place of the quote. */
  replacement: string
  /** Pane scope only: the proposed prose, parsed. */
  paragraphs: Paragraph[]
  /** Pane scope only: paragraph-level comparison against the live pane. */
  diff: DiffRow[]
  /** The anchor no longer resolves; the suggestion cannot be applied. */
  orphaned: boolean
}

export interface PaneDoc {
  pane: Pane
  /** The live `view` entry this pane was reduced from, or null when unwritten. */
  entry: Entry | null
  paragraphs: Paragraph[]
  threads: Thread[]
  /** Amendments awaiting accept or reject, newest first. */
  pending: Amendment[]
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

/** The entry ids a pane cites, in the author's own local order. */
export function citesOf(entry: Entry): string[] {
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

  // Latest live `view` per pane, and every still-undecided proposal against it.
  //
  // Proposals are a LIST rather than one-per-pane: two readers highlighting two
  // different sentences have not collided, and dropping the older of them —
  // which is what keeping only the newest did — silently discarded a suggestion
  // whose author was told it had been recorded.
  const views = new Map<Pane, Entry>()
  const proposals = new Map<Pane, Entry[]>()
  for (const e of entries) {
    if (e.kind !== VIEW_KIND) continue
    const p = paneOf(e)
    if (!p) continue
    if (isProposal(e)) {
      if (decided.has(e.id)) continue
      const list = proposals.get(p)
      if (list) list.push(e)
      else proposals.set(p, [e])
    } else {
      const cur = views.get(p)
      views.set(p, cur ? newer(cur, e) : e)
    }
  }
  // Newest first, deterministically — the same rule `newer` applies pairwise.
  for (const list of proposals.values()) {
    list.sort((a, b) => (newer(a, b) === a ? -1 : 1))
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
    for (const v of proposals.get(p) ?? []) citesOf(v).forEach(number)
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

    // Plain text of each paragraph — the coordinate space anchors resolve in,
    // and the same one `diffParagraphs` compares in.
    const plain = paragraphs.map(paraText)

    const threads: Thread[] = entries
      .filter((e) => e.kind === THREAD_KIND && paneOf(e) === p && !superseded.has(e.id))
      .map((e) => {
        const anchor = anchorOf(meta(e))
        const placed = anchor ? resolveAnchor(plain, anchor) : null
        const raw = meta(e).para
        const n = typeof raw === 'number' && Number.isInteger(raw) ? raw : 0
        // A resolved anchor is authoritative about which paragraph the note is
        // on. Without one — a legacy paragraph-only note — clamp rather than
        // drop: a note that outlived its paragraph is still someone's
        // unanswered question, and the same is true of an orphaned anchor.
        const para = placed
          ? placed.para
          : paragraphs.length === 0
            ? 0
            : Math.min(Math.max(n, 0), paragraphs.length - 1)
        return {
          entry: e,
          para,
          state: stateOf(e),
          ticket: str(meta(e).ticket),
          anchor,
          placed,
          orphaned: anchor !== null && placed === null,
        }
      })

    const pending: Amendment[] = (proposals.get(p) ?? []).map((prop) => {
      const anchor = anchorOf(meta(prop))
      if (anchor) {
        const placed = resolveAnchor(plain, anchor)
        return {
          entry: prop,
          scope: 'range' as const,
          anchor,
          placed,
          replacement: prop.text ?? '',
          paragraphs: [],
          diff: [],
          orphaned: placed === null,
        }
      }
      const next = parseProse(prop.text ?? '', citesOf(prop), byId, numberOf)
      return {
        entry: prop,
        scope: 'pane' as const,
        anchor: null,
        placed: null,
        replacement: '',
        paragraphs: next,
        diff: diffParagraphs(paragraphs, next),
        orphaned: false,
      }
    })

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

/** Threads whose words are gone, so they can be shown somewhere rather than lost. */
export function orphanedThreads(doc: PaneDoc): Thread[] {
  return doc.threads
    .filter((t) => t.orphaned)
    .sort((a, b) => a.entry.created_at.localeCompare(b.entry.created_at))
}

/** Plain text of each paragraph — the coordinate space anchors resolve in. */
export function paneText(doc: PaneDoc): string[] {
  return doc.paragraphs.map(paraText)
}

/** The plain length a run contributes: a citation mark counts as its source form. */
function runLength(r: Run): number {
  return ('text' in r ? r.text : `[${r.cite}]`).length
}

/**
 * Replace [start, end) of a paragraph's plain text with new words.
 *
 * A citation mark that falls inside the replaced range is DROPPED rather than
 * salvaged: the words it supported are the words being replaced, and carrying
 * the mark into prose the source no longer backs would be the one failure this
 * whole citation model exists to prevent.
 */
export function spliceRuns(runs: Run[], start: number, end: number, replacement: string): Run[] {
  const out: Run[] = []
  let pos = 0
  let placed = false
  const emit = (): void => {
    if (placed) return
    if (replacement) out.push({ text: replacement })
    placed = true
  }
  for (const run of runs) {
    const len = runLength(run)
    const rs = pos
    const re = pos + len
    pos = re
    if (re <= start) {
      out.push(run)
      continue
    }
    if (rs >= end) {
      emit()
      out.push(run)
      continue
    }
    if ('text' in run) {
      const head = run.text.slice(0, Math.max(0, start - rs))
      if (head) out.push({ text: head })
      emit()
      const tail = run.text.slice(Math.min(run.text.length, Math.max(0, end - rs)))
      if (tail) out.push({ text: tail })
    } else {
      emit()
    }
  }
  emit()
  return out
}

/**
 * Insert a run at a plain-text offset, splitting a text run if the offset falls
 * inside one. This is how citing a passage works: the mark goes in immediately
 * after the words it supports, and the pane is then re-serialized and appended
 * as a new `view` like any other revision.
 */
export function insertRunAt(runs: Run[], at: number, run: Run): Run[] {
  const out: Run[] = []
  let pos = 0
  let placed = false
  for (const r of runs) {
    const len = runLength(r)
    const start = pos
    const end = pos + len
    pos = end
    if (placed || at > end) {
      out.push(r)
      continue
    }
    if (at === end) {
      out.push(r)
      out.push(run)
      placed = true
      continue
    }
    if (at <= start) {
      out.push(run)
      placed = true
      out.push(r)
      continue
    }
    // Strictly inside. A citation mark is atomic, so an offset inside one goes
    // after it rather than cutting it apart.
    if ('text' in r) {
      const head = r.text.slice(0, at - start)
      const tail = r.text.slice(at - start)
      if (head) out.push({ text: head })
      out.push(run)
      if (tail) out.push({ text: tail })
    } else {
      out.push(r)
      out.push(run)
    }
    placed = true
  }
  if (!placed) out.push(run)
  return out
}

/**
 * Serialize paragraphs back into the source form a `view` entry stores: prose
 * with LOCAL `[1]`-style marks, plus the `cites` array they index.
 *
 * Renumbering from scratch is what keeps an accepted suggestion's citations
 * correct after a mark was dropped — a hole in the numbering would make every
 * later mark point one source too far.
 *
 * One known sharp edge: a literal `[2]` that was left as text because it cited
 * nothing may, after renumbering, land on a real source. Marks have no escape
 * syntax to hide behind, so this is documented rather than defended against; it
 * requires prose that contains a broken citation in the first place.
 */
export function serializeParagraphs(paras: Paragraph[]): { text: string; cites: string[] } {
  const cites: string[] = []
  const seen = new Map<string, number>()
  const blocks = paras
    .map((p) =>
      p.runs
        .map((r) => {
          if ('text' in r) return r.text
          let n = seen.get(r.entry.id)
          if (n === undefined) {
            cites.push(r.entry.id)
            n = cites.length
            seen.set(r.entry.id, n)
          }
          return `[${n}]`
        })
        .join('')
        .trim(),
    )
    .filter(Boolean)
  return { text: blocks.join('\n\n'), cites }
}

/**
 * The `view` an accepted amendment should be appended as.
 *
 * A pane-scoped proposal IS the new view, so it is taken verbatim. A
 * range-scoped one is applied to the live pane — which is why accepting one is
 * still a plain append of complete prose, and why several can be accepted one
 * after another without any of them editing anything.
 *
 * Null when an orphaned range amendment cannot be placed; the caller reports
 * that rather than writing a guess.
 */
export function amendedView(
  doc: PaneDoc,
  am: Amendment,
  citesOfEntry: (e: Entry) => string[] = citesOf,
): { text: string; cites: string[] } | null {
  if (am.scope === 'pane') {
    return { text: am.entry.text ?? '', cites: citesOfEntry(am.entry) }
  }
  if (!am.placed) return null
  const next = doc.paragraphs.map((p, i) =>
    i === am.placed!.para
      ? { ...p, runs: spliceRuns(p.runs, am.placed!.start, am.placed!.end, am.replacement) }
      : p,
  )
  return serializeParagraphs(next)
}
