import { describe, it, expect } from 'vitest'
import { defineStrings, detectLocale, pick, LOCALES } from './i18n'

// A stand-in for a real page table. The compile-time guarantee is the point —
// this file exists for what types cannot see.
const STR = defineStrings({
  en: { board: 'Board', inbox: 'Inbox', empty: 'Nothing here' },
  de: { board: 'Board', inbox: 'Inbox', empty: 'Nichts hier' },
})

describe('string tables', () => {
  it('agrees on every key across locales', () => {
    // The type system already refuses a mismatch; this catches a table built at
    // runtime, which the type cannot inspect.
    const keys = LOCALES.map((l) => Object.keys(STR[l]).sort())
    for (const k of keys) expect(k).toEqual(keys[0])
  })

  it('has no empty translations', () => {
    // A missing translation used to show as a blank label rather than a fallback.
    for (const l of LOCALES) {
      for (const [key, value] of Object.entries(STR[l])) {
        expect(value.length, `${l}.${key} is empty`).toBeGreaterThan(0)
      }
    }
  })

  it('picks the active table', () => {
    expect(pick(STR, 'de').empty).toBe('Nichts hier')
    expect(pick(STR, 'en').empty).toBe('Nothing here')
  })
})

describe('detectLocale', () => {
  it('honours an explicit stored choice', () => {
    expect(detectLocale('de')).toBe('de')
    expect(detectLocale('en')).toBe('en')
  })

  it('falls back to the browser, then to en', () => {
    expect(detectLocale(null)).toBe(navigator.language.toLowerCase().startsWith('de') ? 'de' : 'en')
    expect(detectLocale('klingon')).toBe(detectLocale(null))
  })
})
