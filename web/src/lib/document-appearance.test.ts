import type { CSSProperties } from 'react'
import { describe, expect, it } from 'vitest'
import {
  documentAppearanceStyle, resolveDocumentAppearance, sameDocumentAppearance, validDocumentAppearance,
} from './document-appearance'

describe('document appearance inheritance', () => {
  it('gives older projects the balanced defaults', () => {
    expect(resolveDocumentAppearance()).toEqual({ h1_size: 28, h2_size: 22, h3_size: 18,
      body_size: 16, heading_weight: 600, line_height: 1.6, heading_spacing: 24 })
  })
  it('retains explicit overrides when changing templates and inherits the rest', () => {
    const overrides = { h1_size: 30, heading_spacing: 0 }
    expect(resolveDocumentAppearance({ template: 'strong', overrides })).toMatchObject({ h1_size: 30, h2_size: 24, h3_size: 20, heading_spacing: 0 })
    expect(resolveDocumentAppearance({ template: 'strong', overrides: {} }).h1_size).toBe(32)
  })
  it('keeps explicit values distinct from inherited values even when equal', () => {
    expect(sameDocumentAppearance({ template: 'balanced', overrides: { h1_size: 28 } }, { template: 'balanced', overrides: {} })).toBe(false)
    expect(sameDocumentAppearance({ template: 'strong', overrides: { h1_size: 30, body_size: 18 } },
      { template: 'strong', overrides: { body_size: 18, h1_size: 30 } })).toBe(true)
  })
  it('uses CSS units only for sizes and spacing', () => {
    expect(documentAppearanceStyle({ template: 'strong', overrides: { heading_spacing: 0 } })).toMatchObject({
      '--doc-h1-size': '32px', '--doc-body-size': '16px', '--doc-heading-spacing': '0px', '--doc-line-height': 1.6, '--doc-heading-weight': 600,
    })
  })
  it.each([{ h1_size: 65 }, { body_size: 0 }, { heading_weight: 650 }, { line_height: Infinity }, { heading_spacing: -1 }, { h2_size: NaN }])('rejects invalid overrides %s', (overrides) => {
    expect(validDocumentAppearance({ template: 'balanced', overrides })).toBe(false)
  })
  it('falls back to the template for an override that is not a number yet', () => {
    const resolved = resolveDocumentAppearance({ template: 'strong', overrides: { h1_size: NaN, h2_size: 30 } })
    expect(resolved.h1_size).toBe(32)
    expect(resolved.h2_size).toBe(30)
    expect(documentAppearanceStyle({ template: 'strong', overrides: { h1_size: NaN } })['--doc-h1-size' as keyof CSSProperties]).toBe('32px')
  })
})
