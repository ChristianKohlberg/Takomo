import { describe, expect, it } from 'vitest'
import {
  CITE_ATTR,
  anchorOf,
  makeAnchor,
  plainLengthOf,
  plainOffsetIn,
  resolveAnchor,
  type Anchor,
} from './initiative-anchor'

const P0 = 'We charge a flat fee per seat regardless of usage, which the enterprise tier resents.'
const P1 = 'Metering per request is the alternative, and it is what every competitor does.'

function anchorIn(paras: string[], para: number, quote: string): Anchor {
  const at = (paras[para] ?? '').indexOf(quote)
  expect(at, `fixture must contain ${quote}`).toBeGreaterThanOrEqual(0)
  return makeAnchor(paras, 'business', para, at, at + quote.length)!
}

describe('makeAnchor', () => {
  it('records the quote with context on both sides', () => {
    const a = anchorIn([P0], 0, 'flat fee per seat')
    expect(a.quote).toBe('flat fee per seat')
    expect(P0.startsWith(a.prefix)).toBe(true)
    expect(a.prefix.endsWith('We charge a ')).toBe(true)
    expect(a.suffix.startsWith(' regardless')).toBe(true)
  })

  it('refuses a selection with no words in it', () => {
    expect(makeAnchor([P0], 'business', 0, 4, 4)).toBeNull()
    expect(makeAnchor(['  spaced  '], 'business', 0, 0, 2)).toBeNull()
  })

  it('refuses a paragraph that does not exist', () => {
    expect(makeAnchor([P0], 'business', 7, 0, 4)).toBeNull()
  })

  it('normalises a backwards selection', () => {
    const a = makeAnchor([P0], 'business', 0, 10, 2)!
    expect(a.quote).toBe(P0.slice(2, 10))
  })
})

describe('resolveAnchor', () => {
  it('reports an untouched passage as exact', () => {
    const a = anchorIn([P0, P1], 0, 'flat fee per seat')
    const hit = resolveAnchor([P0, P1], a)!
    expect(hit.how).toBe('exact')
    expect(hit.para).toBe(0)
    expect(P0.slice(hit.start, hit.end)).toBe('flat fee per seat')
  })

  it('follows the quote into another paragraph', () => {
    const a = anchorIn([P0, P1], 0, 'flat fee per seat')
    // The prose was reordered: the same sentence is now second.
    const hit = resolveAnchor([P1, P0], a)!
    expect(hit.para).toBe(1)
    expect(hit.how).toBe('exact')
  })

  it('says moved when the surrounding words changed', () => {
    const a = anchorIn([P0], 0, 'flat fee per seat')
    const revised = 'Today we still charge a flat fee per seat, and nobody likes it.'
    const hit = resolveAnchor([revised], a)!
    expect(hit.how).toBe('moved')
    expect(revised.slice(hit.start, hit.end)).toBe('flat fee per seat')
  })

  it('orphans a note whose words are gone', () => {
    const a = anchorIn([P0], 0, 'flat fee per seat')
    expect(resolveAnchor(['Nothing here resembles the old sentence at all.'], a)).toBeNull()
  })

  it('refuses to guess between two identical occurrences', () => {
    // Context is what disambiguates; strip it and an ambiguous quote must orphan
    // rather than highlight a coin flip.
    const a: Anchor = { pane: 'business', para: 0, quote: 'per seat', prefix: '', suffix: '' }
    expect(resolveAnchor(['per seat and per seat again'], a)).toBeNull()
  })

  it('uses surviving context to pick between two identical occurrences', () => {
    const text = 'billed per seat monthly, or per seat annually'
    const a = makeAnchor([text], 'business', 0, text.lastIndexOf('per seat'), text.lastIndexOf('per seat') + 8)!
    const hit = resolveAnchor([text], a)!
    expect(hit.start).toBe(text.lastIndexOf('per seat'))
  })

  it('survives a paragraph that was merely reflowed', () => {
    const a = anchorIn([P0], 0, 'flat fee per seat')
    const reflowed = P0.replace('flat fee per seat', 'flat fee\n  per   seat')
    const hit = resolveAnchor([reflowed], a)!
    expect(hit.how).toBe('moved')
    expect(reflowed.slice(hit.start, hit.end)).toBe('flat fee\n  per   seat')
  })

  it('resolves against an empty document as orphaned rather than throwing', () => {
    const a = anchorIn([P0], 0, 'flat fee per seat')
    expect(resolveAnchor([], a)).toBeNull()
  })
})

describe('anchorOf', () => {
  it('reads an anchor off entry meta', () => {
    const a = anchorOf({ pane: 'technical', para: 2, quote: 'x', prefix: 'a', suffix: 'b' })!
    expect(a.pane).toBe('technical')
    expect(a.para).toBe(2)
  })

  it('treats a legacy paragraph-only note as having no anchor', () => {
    // Entries written before range anchors existed carry `para` and nothing else.
    // They must keep working as paragraph notes rather than resolving to garbage.
    expect(anchorOf({ pane: 'business', para: 3 })).toBeNull()
    expect(anchorOf(null)).toBeNull()
    expect(anchorOf('nonsense')).toBeNull()
  })

  it('defaults a malformed para to zero instead of NaN', () => {
    expect(anchorOf({ quote: 'x', para: 'two' })!.para).toBe(0)
  })
})

describe('plainOffsetIn', () => {
  /** A paragraph rendered the way the document renders one: text, mark, text. */
  function para(): HTMLElement {
    const p = document.createElement('p')
    p.appendChild(document.createTextNode('We charge a flat fee '))
    const mark = document.createElement('button')
    // The mark DISPLAYS "2" but the prose it came from says "[2]".
    mark.setAttribute(CITE_ATTR, '[2]')
    mark.textContent = '2'
    p.appendChild(mark)
    p.appendChild(document.createTextNode(' per seat.'))
    return p
  }

  it('counts a citation mark as its source form, not its label', () => {
    const p = para()
    expect(plainLengthOf(p)).toBe('We charge a flat fee [2] per seat.'.length)
  })

  it('maps an offset in the trailing text node past the mark', () => {
    const p = para()
    const tail = p.childNodes[2]!
    // Offset 1 into " per seat." is after the space.
    expect(plainOffsetIn(p, tail, 1)).toBe('We charge a flat fee [2] '.length)
  })

  it('maps an offset in the leading text node unchanged', () => {
    const p = para()
    expect(plainOffsetIn(p, p.childNodes[0]!, 3)).toBe(3)
  })

  it('maps a boundary placed on the element itself', () => {
    const p = para()
    expect(plainOffsetIn(p, p, 2)).toBe('We charge a flat fee [2]'.length)
  })

  it('returns the full length for a node outside the paragraph', () => {
    const p = para()
    const stray = document.createTextNode('elsewhere')
    expect(plainOffsetIn(p, stray, 2)).toBe('We charge a flat fee [2] per seat.'.length)
  })
})
