// The markdown renderer — a faithful TypeScript port of src/spa-common.js.
//
// It parses attacker-influenced text: ticket bodies, comments, question text.
// Two properties are load-bearing and must survive any edit here:
//
//   1. It builds DOM NODES. Nothing is ever assigned to innerHTML, so markup in
//      the source text cannot become markup in the page.
//   2. `mdHref` is an allowlist. Only http(s) and mailto produce a link; every
//      other scheme (javascript:, data:, vbscript:) renders as its literal
//      markdown source, so a hostile link cannot hide behind link text.
//
// In the pages this replaced, this file was inlined into each one at a marker
// comment because a browser could not fetch a second file. That constraint is
// gone — it is now an ordinary module, imported — but the reason it was ONE
// copy remains: the same renderer runs on every surface, and a fix landing in
// one copy and not another was a security bug that showed up on one page only.
import { el } from './dom'

const MD_INLINE = /(`[^`]+`|\*\*[^*]+\*\*|\*[^*\n]+\*|_[^_\n]+_|\[[^\]]*\]\()/g
const MD_HEADING = /^(#{1,6})\s+(.*)$/
const MD_RULE = /^(-{3,}|\*{3,}|_{3,})$/
const MD_OL = /^\d+[.)]\s+/
const MD_UL = /^[-*+]\s+/
const MD_QUOTE = /^>\s?/
const MD_FENCE = /^```/

/** Only http(s) and mailto survive. Anything else renders as literal markdown. */
function mdHref(u: string): string | null {
  return /^(https?:\/\/|mailto:)/i.test(u) ? u : null
}

interface LinkTarget {
  url: string
  title: string
  end: number
}

// Scan a link target — everything after `](` — returning `end` just past the
// closing `)`; null when it is not well-formed, which makes the caller print
// the source verbatim.
//
// Hand-rolled with a depth counter rather than a regex, deliberately: balanced
// parens are not a regular language, and the regexes that approximate them are
// exactly the ones that backtrack catastrophically on hostile input. This is
// one left-to-right pass that never rewinds, so its cost is linear.
function mdLinkTarget(s: string, from: number): LinkTarget | null {
  const n = s.length
  let i = from
  let depth = 0
  let url = ''
  while (i < n && (s.charAt(i) === ' ' || s.charAt(i) === '\t')) i++
  while (i < n) {
    const ch = s.charAt(i)
    if (ch === '\n') return null // a target never spans lines
    if (ch === '\\' && (s.charAt(i + 1) === '(' || s.charAt(i + 1) === ')')) {
      url += s.charAt(i + 1) // \( and \) are literals
      i += 2
      continue
    }
    if (ch === '(') {
      depth++
      url += ch
      i++
      continue
    }
    if (ch === ')') {
      if (depth === 0) break // the closing paren
      depth--
      url += ch
      i++
      continue
    }
    if (ch === ' ' || ch === '\t') break // a title may follow
    url += ch
    i++
  }
  if (depth !== 0) return null // unbalanced, so not a link
  while (i < n && (s.charAt(i) === ' ' || s.charAt(i) === '\t')) i++
  let title = ''
  const q = s.charAt(i)
  if (q === '"' || q === "'") {
    i++
    while (i < n && s.charAt(i) !== q) {
      if (s.charAt(i) === '\n') return null
      title += s.charAt(i)
      i++
    }
    if (i >= n) return null // unterminated title
    i++
    while (i < n && (s.charAt(i) === ' ' || s.charAt(i) === '\t')) i++
  }
  if (s.charAt(i) !== ')') return null
  return { url, title, end: i + 1 }
}

function mdInline(parent: Node, raw: unknown): void {
  const s = String(raw == null ? '' : raw)
  let last = 0
  let m: RegExpExecArray | null
  MD_INLINE.lastIndex = 0
  while ((m = MD_INLINE.exec(s)) !== null) {
    if (m.index > last) parent.appendChild(document.createTextNode(s.slice(last, m.index)))
    const tok = m[0]
    const c0 = tok.charAt(0)
    if (c0 === '`') {
      parent.appendChild(el('code', null, tok.slice(1, -1)))
    } else if (tok.indexOf('**') === 0) {
      parent.appendChild(el('b', null, tok.slice(2, -2)))
    } else if (c0 === '*' || c0 === '_') {
      parent.appendChild(el('i', null, tok.slice(1, -1)))
    } else {
      // `tok` is `[text](`, so the target starts just past it. mdHref is still
      // the only thing that decides a scheme is allowed — the wider scan feeds
      // it more URLs, it does not let more of them through.
      const tgt = mdLinkTarget(s, m.index + tok.length)
      const href = tgt ? mdHref(tgt.url) : null
      // Where this construct's source ends. On a refused scheme we still skip
      // the whole parsed target, so `[x](javascript:alert(1))` prints entire
      // rather than having its tail rescanned for inline tokens.
      const end = tgt ? tgt.end : m.index + tok.length
      if (href && tgt) {
        const a = el('a', null, tok.slice(1, -2)) as HTMLAnchorElement
        a.href = href
        a.target = '_blank'
        a.rel = 'noopener noreferrer'
        if (tgt.title) a.title = tgt.title // an attribute value, never parsed as markup
        parent.appendChild(a)
      } else {
        parent.appendChild(document.createTextNode(s.slice(m.index, end)))
      }
      MD_INLINE.lastIndex = end
    }
    last = MD_INLINE.lastIndex
  }
  if (last < s.length) parent.appendChild(document.createTextNode(s.slice(last)))
}

/** `| a | b |` -> ["a","b"], tolerating the optional outer pipes. */
function mdCells(line: string): string[] {
  return line
    .trim()
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map((c) => c.trim())
}

// A table is a pipe row whose NEXT line is a |---|---| separator. Requiring the
// separator keeps prose that merely contains a pipe from becoming a table.
function mdIsTable(lines: string[], i: number): boolean {
  const line = lines[i]
  if (line == null || line.indexOf('|') === -1 || i + 1 >= lines.length) return false
  const sep = (lines[i + 1] ?? '').trim()
  return sep.indexOf('|') !== -1 && /^\|?[\s:|-]+$/.test(sep) && sep.indexOf('-') !== -1
}

// Does line `i` open a block? Used both to dispatch and to end a paragraph, so
// the two can never disagree about where a paragraph stops.
function mdIsBlock(lines: string[], i: number): boolean {
  const t = (lines[i] ?? '').trim()
  return (
    t === '' ||
    MD_FENCE.test(t) ||
    MD_HEADING.test(t) ||
    MD_RULE.test(t) ||
    MD_OL.test(t) ||
    MD_UL.test(t) ||
    MD_QUOTE.test(t) ||
    mdIsTable(lines, i)
  )
}

/**
 * Render markdown to a detached `<div class="md">`. The caller mounts it —
 * see `<Markdown>` in src/components/Markdown.tsx.
 */
export function renderMarkdown(source: unknown, cls?: string | null): HTMLElement {
  const wrap = el('div', cls ? 'md ' + cls : 'md')
  const lines = String(source == null ? '' : source)
    .replace(/\r\n?/g, '\n')
    .split('\n')
  let i = 0
  while (i < lines.length) {
    const t = (lines[i] ?? '').trim()
    if (t === '') {
      i++
      continue
    }

    if (MD_FENCE.test(t)) {
      const lang = t.slice(3).trim()
      const buf: string[] = []
      i++
      while (i < lines.length && !MD_FENCE.test((lines[i] ?? '').trim())) {
        buf.push(lines[i] ?? '')
        i++
      }
      if (i < lines.length) i++ // closing fence
      const pre = el('pre')
      const code = el('code', null, buf.join('\n'))
      if (lang) code.setAttribute('data-lang', lang)
      pre.appendChild(code)
      wrap.appendChild(pre)
      continue
    }

    const h = MD_HEADING.exec(t)
    if (h) {
      // Start at h3: these render inside panels that own h1/h2 already.
      const head = el('h' + Math.min(6, (h[1] ?? '').length + 2))
      mdInline(head, h[2])
      wrap.appendChild(head)
      i++
      continue
    }

    if (MD_RULE.test(t)) {
      wrap.appendChild(el('hr'))
      i++
      continue
    }

    if (mdIsTable(lines, i)) {
      const header = mdCells(lines[i] ?? '')
      i += 2 // header + separator
      const scroll = el('div', 'md-table') // owns the x-overflow
      const table = el('table')
      const thead = el('thead')
      const htr = el('tr')
      header.forEach((c) => {
        const th = el('th')
        mdInline(th, c)
        htr.appendChild(th)
      })
      thead.appendChild(htr)
      table.appendChild(thead)
      const tbody = el('tbody')
      while (i < lines.length && (lines[i] ?? '').trim() !== '' && (lines[i] ?? '').indexOf('|') !== -1) {
        const tr = el('tr')
        mdCells(lines[i] ?? '').forEach((c) => {
          const td = el('td')
          mdInline(td, c)
          tr.appendChild(td)
        })
        tbody.appendChild(tr)
        i++
      }
      table.appendChild(tbody)
      scroll.appendChild(table)
      wrap.appendChild(scroll)
      continue
    }

    if (MD_QUOTE.test(t)) {
      const bq = el('blockquote')
      let firstQ = true
      while (i < lines.length && MD_QUOTE.test((lines[i] ?? '').trim())) {
        if (!firstQ) bq.appendChild(el('br'))
        mdInline(bq, (lines[i] ?? '').trim().replace(MD_QUOTE, ''))
        firstQ = false
        i++
      }
      wrap.appendChild(bq)
      continue
    }

    if (MD_OL.test(t) || MD_UL.test(t)) {
      const ordered = MD_OL.test(t)
      const list = el(ordered ? 'ol' : 'ul')
      const re = ordered ? MD_OL : MD_UL
      while (i < lines.length && re.test((lines[i] ?? '').trim())) {
        const li = el('li')
        mdInline(li, (lines[i] ?? '').trim().replace(re, ''))
        list.appendChild(li)
        i++
      }
      wrap.appendChild(list)
      continue
    }

    // Paragraph. A single newline becomes a <br> rather than being reflowed:
    // agents hand-format these, and silently joining their lines changes what
    // they wrote.
    const p = el('p')
    let first = true
    while (i < lines.length && !mdIsBlock(lines, i)) {
      if (!first) p.appendChild(el('br'))
      mdInline(p, (lines[i] ?? '').trim())
      first = false
      i++
    }
    wrap.appendChild(p)
  }
  return wrap
}
