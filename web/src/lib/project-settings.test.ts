import { describe, it, expect, vi, beforeEach } from 'vitest'
import {
  STYLE_MAX,
  saveProjectSettings,
  saveBlockReason,
  settingsFrom,
  type ProjectSettings,
} from './project-settings'

vi.mock('./api', () => ({ api: vi.fn().mockResolvedValue({}) }))
import { api } from './api'
beforeEach(() => vi.clearAllMocks())

const W = { readOnly: 'read only', over: 'too long' }

const base: ProjectSettings = {
  documentAppearance: { template: 'balanced', overrides: {} },
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
      documentAppearance: { template: 'balanced', overrides: {} },
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

  it('counts characters, not UTF-16 code units, against the server limit', () => {
    const emoji = '😀'
    const near = emoji.repeat(STYLE_MAX)
    expect(saveBlockReason({ ...base, style: near }, false, W)).toBe('')
    expect(saveBlockReason({ ...base, style: near + emoji }, false, W)).toBe(W.over)
  })

  it('permits an unchanged, empty form', () => {
    expect(saveBlockReason(base, false, W)).toBe('')
  })
})


describe('document appearance settings persistence', () => {
  it('saves only changed appearance and retains explicit overrides', async () => {
    const documentAppearance = { template: 'strong' as const, overrides: { h1_size: 36 } }
    expect(await saveProjectSettings('token', 'demo', { ...base, documentAppearance }, base)).toBe(1)
    expect(api).toHaveBeenCalledWith('token', '/projects/demo/document-appearance', expect.objectContaining({
      method: 'PUT', body: JSON.stringify(documentAppearance),
    }))
  })
  it('does not save default appearance on older projects or unchanged settings', async () => {
    expect(await saveProjectSettings('token', 'demo', base, { ...base, documentAppearance: undefined })).toBe(0)
    expect(api).not.toHaveBeenCalled()
  })
  it('removes an override on the wire when reset', async () => {
    await saveProjectSettings('token', 'demo', base, { ...base, documentAppearance: { template: 'balanced', overrides: { h1_size: 36 } } })
    expect(api).toHaveBeenCalledWith('token', '/projects/demo/document-appearance', expect.objectContaining({
      body: JSON.stringify(base.documentAppearance),
    }))
  })
  it('blocks a save while a numeric field is cleared mid-edit', () => {
    const cleared = { ...base, documentAppearance: { template: 'balanced' as const, overrides: { h2_size: NaN } } }
    expect(saveBlockReason(cleared, false, { ...W, appearanceInvalid: 'invalid' })).toBe('invalid')
  })
  it('blocks invalid appearance before making any settings writes', async () => {
    const next = { ...base, language: 'de', documentAppearance: { template: 'balanced' as const, overrides: { h1_size: 100 } } }
    await expect(saveProjectSettings('token', 'demo', next, base)).rejects.toThrow('Invalid document appearance')
    expect(api).not.toHaveBeenCalled()
    expect(saveBlockReason(next, false, { ...W, appearanceInvalid: 'invalid' })).toBe('invalid')
  })
})
