// The typeahead is the most-mounted component in the app — five mounts across
// two surfaces — and until now it had no test at all. That is how it shipped
// telling the reader "12 matches" when there were four hundred.
import { describe, it, expect, afterEach, vi } from 'vitest'
import { render, screen, cleanup, fireEvent } from '@testing-library/react'
import { Typeahead } from './Typeahead'

afterEach(cleanup)

const LABELS = {
  all: 'All tickets',
  placeholder: 'Filter by ticket',
  clear: 'Clear',
  noMatch: 'No match for “{q}”',
  count: '{n} matches',
  count1: '1 match',
  countTruncated: 'showing {shown} of {n}',
}

/** More options than the popup will render, all matching the same query. */
const many = (n: number) =>
  Array.from({ length: n }, (_, i) => ({ id: `demo-${i}`, title: `Migrate service ${i}` }))

function open(options: { id: string; title?: string }[], query: string) {
  const r = render(
    <Typeahead id="ta" options={options} value="" onChange={vi.fn()} labels={LABELS} />,
  )
  const input = screen.getByRole('combobox')
  fireEvent.focus(input)
  if (query) fireEvent.change(input, { target: { value: query } })
  return r
}

describe('the match count', () => {
  it('reports the TOTAL, not the number it happened to render', () => {
    // The defect: `.filter().slice(0, 12)` then counting the slice. A reader
    // told "12 matches" reasonably concludes the list in front of them is the
    // whole result set, and stops typing.
    open(many(400), 'migrate')
    expect(screen.getByText('showing 12 of 400')).toBeTruthy()
  })

  it('says "1 match" for exactly one', () => {
    open([{ id: 'demo-1', title: 'Only one' }], 'only')
    expect(screen.getByText('1 match')).toBeTruthy()
  })

  it('uses the plain count when nothing was truncated', () => {
    open(many(3), 'migrate')
    expect(screen.getByText('3 matches')).toBeTruthy()
  })
})

describe('ranking', () => {
  const OPTS = [
    { id: 'demo-zzz', title: 'Something about auth in the title' },
    { id: 'demo-auth-2', title: 'Second' },
    { id: 'auth', title: 'Exact id' },
    { id: 'demo-auth-1', title: 'First' },
  ]

  /** The rendered options, minus the "All tickets" escape hatch that leads them. */
  const results = () =>
    screen
      .getAllByRole('option')
      .map((o) => o.textContent ?? '')
      .filter((t) => t !== LABELS.all)

  it('puts an exact id first, then id prefixes, then title matches', () => {
    open(OPTS, 'auth')
    const rendered = results()
    // Exact id wins outright.
    expect(rendered[0]).toContain('auth')
    expect(rendered[0]).toContain('Exact id')
    // The title-only match sinks below the id matches.
    expect(rendered[rendered.length - 1]).toContain('Something about auth')
  })

  it('keeps equal-ranked options in their original order', () => {
    // A filter that reshuffles equal results as you type is worse than one that
    // does not, so the sort has to be stable.
    open(OPTS, 'auth')
    const rendered = results()
    const first = rendered.findIndex((r) => r.includes('demo-auth-1'))
    const second = rendered.findIndex((r) => r.includes('demo-auth-2'))
    expect(second).toBeLessThan(first) // demo-auth-2 precedes demo-auth-1 in OPTS
  })
})

describe('the popup', () => {
  it('is a real listbox of options, and announces the active one', () => {
    open(many(5), 'migrate')
    const input = screen.getByRole('combobox')
    expect(input.getAttribute('aria-expanded')).toBe('true')
    expect(screen.getByRole('listbox')).toBeTruthy()
    expect(input.getAttribute('aria-activedescendant')).toBeTruthy()
  })

  it('says so when nothing matches, rather than showing an empty box', () => {
    open(many(5), 'nothing-matches-this')
    expect(screen.getByText('No match for “nothing-matches-this”')).toBeTruthy()
  })
})
