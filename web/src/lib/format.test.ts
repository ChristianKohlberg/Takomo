import { describe, it, expect } from 'vitest'
import { fmtAge, fmtBytes, fmtDuration, splitList, localInputToRfc3339, charCount } from './format'

describe('fmtAge', () => {
  const now = Date.parse('2026-08-08T12:00:00Z')
  const ago = (ms: number) => new Date(now - ms).toISOString()

  it('picks the coarsest unit that fits', () => {
    expect(fmtAge(ago(45_000), now)).toBe('45s')
    expect(fmtAge(ago(12 * 60_000), now)).toBe('12m')
    expect(fmtAge(ago(3 * 3_600_000), now)).toBe('3h')
    expect(fmtAge(ago(9 * 86_400_000), now)).toBe('9d')
  })

  it('clamps a future timestamp to 0 rather than showing a negative age', () => {
    expect(fmtAge(new Date(now + 60_000).toISOString(), now)).toBe('0s')
  })

  it('returns an em dash for missing or unparseable input', () => {
    expect(fmtAge(null, now)).toBe('—')
    expect(fmtAge(undefined, now)).toBe('—')
    expect(fmtAge('', now)).toBe('—')
    expect(fmtAge('not a date', now)).toBe('—')
  })
})

describe('fmtBytes', () => {
  it('keeps small sizes in their real unit', () => {
    // The rollup's `megabytes` would render a 300-byte note as "0 MB".
    expect(fmtBytes(300)).toBe('300 B')
    expect(fmtBytes(0)).toBe('0 B')
    expect(fmtBytes(null)).toBe('0 B')
  })

  it('switches precision by magnitude', () => {
    expect(fmtBytes(2048)).toBe('2.0 KB')
    expect(fmtBytes(20480)).toBe('20 KB')
    expect(fmtBytes(2 * 1024 * 1024)).toBe('2.00 MB')
    expect(fmtBytes(20 * 1024 * 1024)).toBe('20.0 MB')
  })
})

describe('charCount', () => {
  it('counts Unicode scalar values like Rust str.chars().count()', () => {
    // UTF-16 code units vs characters: one emoji is one char, two code units.
    const emoji = '😀'
    expect(emoji.length).toBe(2)
    expect(charCount(emoji)).toBe(1)
    expect(charCount('ab')).toBe(2)
  })
})

describe('splitList', () => {
  it('trims and drops empties', () => {
    expect(splitList('a, b ,,c ')).toEqual(['a', 'b', 'c'])
    expect(splitList('')).toEqual([])
    expect(splitList(null)).toEqual([])
    expect(splitList('  ,  ')).toEqual([])
  })
})

describe('localInputToRfc3339', () => {
  it('adds the browser zone to a datetime-local value', () => {
    const out = localInputToRfc3339('2026-07-01T09:00')
    expect(out).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z$/)
    // Same instant as the local reading, whatever the runner's zone is.
    expect(Date.parse(out!)).toBe(new Date('2026-07-01T09:00').getTime())
  })

  it('returns null for empty or junk, so the caller omits the field', () => {
    // The server refuses a non-RFC-3339 value rather than guessing; sending
    // nothing is correct, sending "" is a 422.
    expect(localInputToRfc3339('')).toBeNull()
    expect(localInputToRfc3339('   ')).toBeNull()
    expect(localInputToRfc3339('yesterday')).toBeNull()
  })
})

describe('fmtDuration', () => {
  it('uses the same buckets as fmtAge, from seconds', () => {
    expect(fmtDuration(45)).toBe('45s')
    expect(fmtDuration(60)).toBe('1m')
    expect(fmtDuration(3599)).toBe('59m')
    expect(fmtDuration(3600)).toBe('1h')
    expect(fmtDuration(86_399)).toBe('23h')
    expect(fmtDuration(86_400)).toBe('1d')
    expect(fmtDuration(9 * 86_400)).toBe('9d')
  })

  // The server computes these, so no browser clock is involved and a negative
  // value can only be nonsense — clamp rather than print "-3s".
  it('clamps a negative duration and dashes a missing one', () => {
    expect(fmtDuration(-5)).toBe('0s')
    expect(fmtDuration(0)).toBe('0s')
    expect(fmtDuration(null)).toBe('—')
    expect(fmtDuration(undefined)).toBe('—')
    expect(fmtDuration(Number.NaN)).toBe('—')
  })
})
