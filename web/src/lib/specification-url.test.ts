import { describe, expect, it } from 'vitest'
import { specificationFocus, specificationLink } from './specification-url'

describe('specification links', () => {
  it('round-trips section focus beside the selection', () => {
    const link = specificationLink('proj one', 'document', 'mn-7-4', 'mn-7')
    expect(link).toBe('/projects/proj%20one/specification?view=document&section=mn-7-4&focus=mn-7')
    const search = link.slice(link.indexOf('?'))
    expect(specificationFocus(search)).toBe('mn-7')
    expect(new URLSearchParams(search).get('section')).toBe('mn-7-4')
  })
  it('keeps links without focus unchanged and never carries focus to other views', () => {
    expect(specificationLink('p', 'document', 's')).toBe('/projects/p/specification?view=document&section=s')
    expect(specificationLink('p', 'document', null, null)).toBe('/projects/p/specification?view=document')
    expect(specificationLink('p', 'map', 's', 'f')).toBe('/projects/p/specification?view=map&section=s')
    expect(specificationFocus('?view=document&focus=')).toBeNull()
    expect(specificationFocus('?view=document')).toBeNull()
  })
})
