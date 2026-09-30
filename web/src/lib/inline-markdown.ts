// Inline markdown inside one block: `**bold**`, `*italic*`/`_italic_`,
// `~~strike~~`, `` `code` `` and `[label](url)`.
//
// This is a CONTRACT, not a convenience, because it has a twin: the server
// applies an accepted proposal itself (`src/api/proposal_apply.rs`), and a
// proposal must land as the same marks whichever side applies it. So the
// grammar here is deliberately small, closed and spelled out — not CommonMark —
// and the Rust port follows it step for step. `tests/fixtures/proposal-markdown.json`
// is generated from this code and asserted by the Rust tests; change one side
// and the other goes red.
//
// The grammar, over code points:
//
// 1. Tokenize. `\` before ASCII punctuation is that character, literally (it can
//    never be a delimiter). A run of N backticks opens a code span closed by the
//    next run of exactly N backticks; its content is taken verbatim — no escapes,
//    no marks. An unmatched run is literal backticks.
// 2. Links. A `[` whose matching `]` (brackets nest) is directly followed by
//    `(`, then a destination with no whitespace, then `)`, is a link. The label is
//    parsed for emphasis but not for further links. A destination with a scheme
//    other than http, https or mailto is not a link.
// 3. Emphasis. `**`/`__` bold, `~~` strike, `*`/`_` italic. An opener is at the
//    START of a delimiter run for a double, at the END of one for a single, and
//    must be followed by a non-space; a closer is the LAST two of a run for a
//    double, the FIRST of a run for a single, and must follow a non-space. `_`
//    additionally never opens after, or closes before, a letter or digit, so
//    `snake_case_name` stays text. The FIRST valid closer wins; an opener with no
//    closer is literal.
//
// Anything that does not match is literal text. That is the rule a proposal
// reviewer relies on: a stray `*` shows as a `*`, it never swallows a sentence.

export type InlineMarkType = 'link' | 'bold' | 'italic' | 'strike' | 'code'

export interface InlineMark {
  type: InlineMarkType
  /** Only for `link`. */
  href?: string
}

export interface InlineRun {
  text: string
  /** Sorted by `MARK_ORDER`. */
  marks: InlineMark[]
}

/** The order marks are listed (and, on the read side, opened) in. */
export const MARK_ORDER: readonly InlineMarkType[] = ['link', 'bold', 'italic', 'strike', 'code']

const ESCAPABLE = new Set(Array.from('!"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~'))
const SPACE = new Set([' ', '\t', '\n', '\r', '\f', '\v'])
const ALNUM = /^[\p{Alphabetic}\p{N}]$/u

type Tok = { ch: string; literal: boolean; code?: undefined } | { code: string; ch?: undefined; literal?: undefined }

function tokenize(chars: string[]): Tok[] {
  // Backtick runs, and for each the next run of the same length — so an
  // unmatched run costs one lookup instead of a scan to the end of the text.
  const runs: { start: number; len: number }[] = []
  for (let i = 0; i < chars.length;) {
    if (chars[i] === '`') {
      let e = i
      while (e < chars.length && chars[e] === '`') e++
      runs.push({ start: i, len: e - i })
      i = e
    } else i++
  }
  const nextSame: number[] = new Array(runs.length).fill(-1)
  const lastByLen = new Map<number, number>()
  for (let k = runs.length - 1; k >= 0; k--) {
    nextSame[k] = lastByLen.get(runs[k]!.len) ?? -1
    lastByLen.set(runs[k]!.len, k)
  }
  const runAt = new Map<number, number>()
  runs.forEach((r, k) => runAt.set(r.start, k))

  const out: Tok[] = []
  let i = 0
  while (i < chars.length) {
    const c = chars[i]!
    if (c === '\\' && i + 1 < chars.length && ESCAPABLE.has(chars[i + 1]!)) {
      out.push({ ch: chars[i + 1]!, literal: true })
      i += 2
      continue
    }
    const k = runAt.get(i)
    if (k !== undefined) {
      const run = runs[k]!
      const close = nextSame[k]!
      if (close >= 0) {
        const end = runs[close]!.start
        out.push({ code: chars.slice(run.start + run.len, end).join('') })
        i = end + run.len
      } else {
        for (let n = 0; n < run.len; n++) out.push({ ch: '`', literal: true })
        i += run.len
      }
      continue
    }
    out.push({ ch: c, literal: false })
    i++
  }
  return out
}

const isDelim = (t: Tok | undefined, c: string) => !!t && t.ch === c && !t.literal
const isSpace = (t: Tok | undefined) => !!t && t.ch !== undefined && SPACE.has(t.ch)
const isAlnum = (t: Tok | undefined) => !!t && t.ch !== undefined && ALNUM.test(t.ch)

/** Whether a destination may become a link. A scheme must be http(s) or mailto. */
export function safeHref(href: string): boolean {
  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(href)
  return !scheme || ['http', 'https', 'mailto'].includes(scheme[1]!.toLowerCase())
}

interface Delim { text: string; c: string; mark: InlineMarkType }
const DOUBLES: Delim[] = [
  { text: '**', c: '*', mark: 'bold' },
  { text: '__', c: '_', mark: 'bold' },
  { text: '~~', c: '~', mark: 'strike' },
]
const SINGLES: Delim[] = [
  { text: '*', c: '*', mark: 'italic' },
  { text: '_', c: '_', mark: 'italic' },
]

function sameMark(a: InlineMark, b: InlineMark): boolean {
  return a.type === b.type && (a.href ?? null) === (b.href ?? null)
}

function withMark(marks: InlineMark[], mark: InlineMark): InlineMark[] {
  // An inner mark of the same type wins (a link label cannot hold a link, so
  // this only ever meets emphasis doubled up, which is the same mark anyway).
  if (marks.some((m) => m.type === mark.type)) return marks
  return [...marks, mark].sort((a, b) => MARK_ORDER.indexOf(a.type) - MARK_ORDER.indexOf(b.type))
}

/** Parse one block's inline markdown into runs of text with marks. */
export function parseInline(text: string): InlineRun[] {
  const T = tokenize(Array.from(text))
  const m = T.length

  // Valid closer positions per delimiter, ascending. Validity only depends on
  // neighbours, so it is computed once.
  const closers = new Map<string, number[]>()
  for (const d of DOUBLES) {
    const list: number[] = []
    for (let j = 1; j + 1 < m; j++) {
      if (!isDelim(T[j], d.c) || !isDelim(T[j + 1], d.c) || isDelim(T[j + 2], d.c)) continue
      if (isSpace(T[j - 1])) continue
      if (d.c === '_' && isAlnum(T[j + 2])) continue
      list.push(j)
    }
    closers.set(d.text, list)
  }
  for (const d of SINGLES) {
    const list: number[] = []
    for (let j = 1; j < m; j++) {
      if (!isDelim(T[j], d.c) || isDelim(T[j - 1], d.c) || isSpace(T[j - 1])) continue
      if (d.c === '_' && isAlnum(T[j + 1])) continue
      list.push(j)
    }
    closers.set(d.text, list)
  }
  // Matching `]` for each `[`.
  const match = new Map<number, number>()
  const stack: number[] = []
  T.forEach((t, i) => {
    if (isDelim(t, '[')) stack.push(i)
    else if (isDelim(t, ']') && stack.length) match.set(stack.pop()!, i)
  })
  // First index at or after p where a link destination must stop.
  const stop: number[] = new Array(m + 1).fill(m)
  for (let p = m - 1; p >= 0; p--) {
    const t = T[p]!
    stop[p] = t.code !== undefined || isSpace(t) || isDelim(t, ')') ? p : stop[p + 1]!
  }

  const firstCloser = (d: Delim, from: number, end: number): number => {
    const list = closers.get(d.text)!
    let lo = 0, hi = list.length
    while (lo < hi) {
      const mid = (lo + hi) >> 1
      if (list[mid]! < from) lo = mid + 1
      else hi = mid
    }
    const j = lo < list.length ? list[lo]! : -1
    return j >= 0 && j + d.text.length <= end ? j : -1
  }

  const parse = (start: number, end: number, links: boolean): InlineRun[] => {
    const runs: InlineRun[] = []
    let buf = ''
    const flush = () => { if (buf) runs.push({ text: buf, marks: [] }); buf = '' }
    const nest = (inner: InlineRun[], mark: InlineMark) => {
      flush()
      for (const r of inner) runs.push({ text: r.text, marks: withMark(r.marks, mark) })
    }
    let i = start
    outer: while (i < end) {
      const t = T[i]!
      if (t.code !== undefined) {
        flush()
        runs.push({ text: t.code, marks: [{ type: 'code' }] })
        i++
        continue
      }
      if (links && isDelim(t, '[')) {
        const k = match.get(i)
        if (k !== undefined && k > i + 1 && k + 1 < end && isDelim(T[k + 1], '(')) {
          const h = stop[k + 2]!
          if (h < end && h > k + 2 && isDelim(T[h], ')')) {
            const href = T.slice(k + 2, h).map((x) => x.ch).join('')
            if (safeHref(href)) {
              nest(parse(i + 1, k, false), { type: 'link', href })
              i = h + 1
              continue
            }
          }
        }
      }
      if (!t.literal && (t.ch === '*' || t.ch === '_' || t.ch === '~')) {
        for (const d of DOUBLES) {
          if (d.c !== t.ch || !isDelim(T[i + 1], d.c) || isDelim(T[i - 1], d.c)) continue
          if (i + 2 >= end || isSpace(T[i + 2])) continue
          if (d.c === '_' && isAlnum(T[i - 1])) continue
          const j = firstCloser(d, i + 3, end)
          if (j < 0) continue
          nest(parse(i + 2, j, links), { type: d.mark })
          i = j + 2
          continue outer
        }
        for (const d of SINGLES) {
          if (d.c !== t.ch || i + 1 >= end || isDelim(T[i + 1], d.c) || isSpace(T[i + 1])) continue
          if (d.c === '_' && isAlnum(T[i - 1])) continue
          const j = firstCloser(d, i + 2, end)
          if (j < 0) continue
          nest(parse(i + 1, j, links), { type: d.mark })
          i = j + 1
          continue outer
        }
      }
      buf += t.ch
      i++
    }
    flush()
    return runs
  }

  // Adjacent runs with the same marks are one run.
  const out: InlineRun[] = []
  for (const r of parse(0, m, true)) {
    const last = out[out.length - 1]
    if (last && last.marks.length === r.marks.length && last.marks.every((mk, n) => sameMark(mk, r.marks[n]!)))
      last.text += r.text
    else out.push({ text: r.text, marks: [...r.marks] })
  }
  return out
}
