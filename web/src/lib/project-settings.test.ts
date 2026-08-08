import { describe, it, expect } from 'vitest'
import {
  STYLE_MAX,
  saveBlockReason,
  settingsFrom,
  type ProjectSettings,
} from './project-settings'

const W = { readOnly: 'read only', over: 'too long' }

const base: ProjectSettings = {
  language: '',
  style: '',
  ttl: '',
  claimTtl: '',
  maxClaimTtl: '',
}

describe('settingsFrom', () => {
  it('maps an unset field to the empty string, not to "null"', () => {
    // The form binds these to inputs; a literal "null" in a text box is the
    // classic version of this bug.
    expect(settingsFrom({ id: 'demo' })).toEqual(base)
    expect(settingsFrom(undefined)).toEqual(base)
  })

  it('stringifies numbers so the inputs stay controlled', () => {
    const s = settingsFrom({
      id: 'demo',
      question_language: 'de',
      style_guide: 'Terse.',
      answer_link_ttl_seconds: 604800,
      claim_ttl_seconds: 900,
      max_claim_ttl_seconds: 3600,
    })
    expect(s).toEqual({
      language: 'de',
      style: 'Terse.',
      ttl: '604800',
      claimTtl: '900',
      maxClaimTtl: '3600',
    })
  })

  it('keeps a zero, which is a real value and not an absent one', () => {
    expect(settingsFrom({ id: 'demo', claim_ttl_seconds: 0 }).claimTtl).toBe('0')
  })
})

describe('saveBlockReason', () => {
  it('refuses a read-only viewer before anything else', () => {
    expect(saveBlockReason(base, true, W)).toBe(W.readOnly)
  })

  it('refuses an over-long style guide', () => {
    const long = { ...base, style: 'x'.repeat(STYLE_MAX + 1) }
    expect(saveBlockReason(long, false, W)).toBe(W.over)
    expect(saveBlockReason({ ...base, style: 'x'.repeat(STYLE_MAX) }, false, W)).toBe('')
  })

  it('measures the TRIMMED style, so trailing whitespace cannot block a save', () => {
    const padded = { ...base, style: 'x'.repeat(STYLE_MAX) + '   ' }
    expect(saveBlockReason(padded, false, W)).toBe('')
  })

  it('permits an unchanged, empty form', () => {
    expect(saveBlockReason(base, false, W)).toBe('')
  })
})
