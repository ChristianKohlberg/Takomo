// Handing a section between the two views of one plan.
import { describe, expect, it } from 'vitest'

import { mapLink, planLink, readPlanFocus } from './plan-url'

describe('planLink', () => {
  it('carries the section the map was looking at', () => {
    expect(planLink('mn-7')).toBe('/documents#n=mn-7')
  })

  it('is the plain plan when nothing is selected', () => {
    expect(planLink(null)).toBe('/documents')
    expect(planLink()).toBe('/documents')
  })

  it('escapes an id rather than letting it end the fragment', () => {
    expect(planLink('mn a&b')).toBe('/documents#n=mn%20a%26b')
  })
})

describe('readPlanFocus', () => {
  it('reads back what planLink wrote', () => {
    expect(readPlanFocus(planLink('mn-7'))).toBe('mn-7')
    expect(readPlanFocus(planLink('mn a&b'))).toBe('mn a&b')
  })

  it('takes the hash with or without its #', () => {
    expect(readPlanFocus('#n=mn-1')).toBe('mn-1')
    expect(readPlanFocus('n=mn-1')).toBe('mn-1')
  })

  it('is no ask rather than an error when there is nothing to read', () => {
    expect(readPlanFocus('')).toBeNull()
    expect(readPlanFocus('#')).toBeNull()
    expect(readPlanFocus('#m=mm-1')).toBeNull()
    expect(readPlanFocus('#n=')).toBeNull()
    expect(readPlanFocus('#n=%20')).toBeNull()
  })
})

describe('mapLink', () => {
  it('names the map as well as the section, because /mindmaps opens what it is given', () => {
    expect(mapLink('mm-1', 'mn-7')).toBe('/mindmaps#m=mm-1&n=mn-7')
    expect(mapLink('mm-1')).toBe('/mindmaps#m=mm-1')
  })
})
