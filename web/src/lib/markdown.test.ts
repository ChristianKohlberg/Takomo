// The tests the old pages could not have: `spa-common.js` was only ever checked
// by eslint (undefined names, dead bindings) and by eye. This renderer parses
// attacker-influenced text, so the properties below are the ones that actually
// matter — and none of them were verified anywhere before now.
import { describe, it, expect } from 'vitest'
import { renderMarkdown } from './markdown'

const html = (src: string) => renderMarkdown(src).innerHTML
const el = (src: string) => renderMarkdown(src)

describe('markdown: markup can never come from the source text', () => {
  it('renders raw HTML as text, not as elements', () => {
    const out = el('<script>alert(1)</script> and <b>bold</b>')
    expect(out.querySelector('script')).toBeNull()
    // The <b> in the SOURCE must not become an element — only markdown syntax
    // produces elements.
    expect(out.querySelectorAll('b')).toHaveLength(0)
    expect(out.textContent).toContain('<script>alert(1)</script>')
  })

  it('escapes markup inside a fenced code block', () => {
    const out = el('```\n<img src=x onerror=alert(1)>\n```')
    expect(out.querySelector('img')).toBeNull()
    expect(out.querySelector('code')?.textContent).toBe('<img src=x onerror=alert(1)>')
  })

  it('escapes markup inside table cells', () => {
    const out = el('| a |\n|---|\n| <b>x</b> |')
    expect(out.querySelectorAll('td b')).toHaveLength(0)
    expect(out.querySelector('td')?.textContent).toBe('<b>x</b>')
  })
})

describe('markdown: the link scheme allowlist', () => {
  it.each([
    'javascript:alert(1)',
    'JaVaScRiPt:alert(1)',
    'data:text/html;base64,PHNjcmlwdD4=',
    'vbscript:msgbox(1)',
    'file:///etc/passwd',
    '/relative/path',
  ])('refuses %s and prints the source verbatim', (scheme) => {
    const src = `[click](${scheme})`
    const out = el(src)
    expect(out.querySelector('a')).toBeNull()
    expect(out.textContent).toBe(src)
  })

  it.each(['https://example.com/x', 'http://example.com', 'mailto:a@b.c'])(
    'allows %s',
    (url) => {
      const a = el(`[click](${url})`).querySelector('a')
      expect(a).not.toBeNull()
      expect(a!.getAttribute('href')).toBe(url)
      expect(a!.getAttribute('target')).toBe('_blank')
      expect(a!.getAttribute('rel')).toBe('noopener noreferrer')
      expect(a!.textContent).toBe('click')
    },
  )

  it('does not rescan the tail of a refused link for inline tokens', () => {
    // The whole construct must print literally; if the tail were rescanned the
    // `**b**` inside it would become an element.
    const out = el('[x](javascript:alert(**b**))')
    expect(out.querySelectorAll('b')).toHaveLength(0)
    expect(out.textContent).toBe('[x](javascript:alert(**b**))')
  })
})

describe('markdown: the hand-rolled link-target scanner', () => {
  it('accepts balanced parens inside a URL', () => {
    const a = el('[wiki](https://en.wikipedia.org/wiki/Takomo_(disambiguation))').querySelector('a')
    expect(a?.getAttribute('href')).toBe('https://en.wikipedia.org/wiki/Takomo_(disambiguation)')
  })

  it('treats an unbalanced target as not-a-link', () => {
    const src = '[x](https://e.com/(a'
    expect(el(src).querySelector('a')).toBeNull()
    expect(el(src).textContent).toBe(src)
  })

  it('reads an optional quoted title into an attribute', () => {
    const a = el('[x](https://e.com "the title")').querySelector('a')
    expect(a?.getAttribute('href')).toBe('https://e.com')
    expect(a?.getAttribute('title')).toBe('the title')
  })

  it('refuses a target that spans lines', () => {
    expect(el('[x](https://e.com\n)').querySelector('a')).toBeNull()
  })

  it('terminates on adversarial nesting rather than backtracking', () => {
    // A regex approximating balanced parens would hang here; the linear scanner
    // must return promptly. 5k nested parens, unbalanced.
    const src = '[x](' + '('.repeat(5000)
    const started = performance.now()
    expect(el(src).querySelector('a')).toBeNull()
    expect(performance.now() - started).toBeLessThan(1000)
  })
})

describe('markdown: block structure', () => {
  it('renders inline code, bold and italic', () => {
    expect(html('`c` **b** *i* _u_')).toContain('<code>c</code>')
    expect(html('`c` **b** *i* _u_')).toContain('<b>b</b>')
    expect(el('`c` **b** *i* _u_').querySelectorAll('i')).toHaveLength(2)
  })

  it('starts headings at h3, because panels own h1/h2', () => {
    expect(el('# top').querySelector('h3')).not.toBeNull()
    expect(el('### three').querySelector('h5')).not.toBeNull()
    expect(el('###### six').querySelector('h6')).not.toBeNull()
  })

  it('requires a separator row before treating pipes as a table', () => {
    expect(el('| not | a table |').querySelector('table')).toBeNull()
    expect(el('a | b in prose').querySelector('table')).toBeNull()
    expect(el('| a | b |\n|---|---|\n| 1 | 2 |').querySelector('table')).not.toBeNull()
  })

  it('keeps a fence language as a data attribute', () => {
    expect(el('```rust\nfn main() {}\n```').querySelector('code')?.getAttribute('data-lang')).toBe('rust')
  })

  it('turns a single newline into <br> rather than reflowing', () => {
    // Agents hand-format these; joining their lines changes what they wrote.
    expect(el('one\ntwo').querySelectorAll('br')).toHaveLength(1)
  })

  it('renders ordered and unordered lists', () => {
    expect(el('- a\n- b').querySelectorAll('ul li')).toHaveLength(2)
    expect(el('1. a\n2. b').querySelectorAll('ol li')).toHaveLength(2)
  })

  it('renders a blockquote with line breaks', () => {
    const bq = el('> a\n> b').querySelector('blockquote')
    expect(bq).not.toBeNull()
    expect(bq!.querySelectorAll('br')).toHaveLength(1)
  })

  it('survives empty and null input', () => {
    expect(renderMarkdown('').textContent).toBe('')
    expect(renderMarkdown(null).textContent).toBe('')
    expect(renderMarkdown(undefined).textContent).toBe('')
  })

  it('takes an extra class while keeping md', () => {
    expect(renderMarkdown('x', 'compact').className).toBe('md compact')
    expect(renderMarkdown('x').className).toBe('md')
  })
})
