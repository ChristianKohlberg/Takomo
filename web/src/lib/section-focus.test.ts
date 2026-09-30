import { describe, expect, it } from 'vitest'
import { planSections } from './plan-sections'
import { inFocus, outsideFocus, sectionFocusScope } from './section-focus'

const node = (id: string, parent: string | null, position: number) => ({ id, parent, order: String(position), title: id.toUpperCase(), position })
const sections = planSections([node('a', null, 0), node('a1', 'a', 0), node('a1x', 'a1', 0), node('a2', 'a', 1), node('b', null, 1)])

describe('section focus scope', () => {
  it('holds the subtree with real numbers and the ancestor trail', () => {
    const scope = sectionFocusScope(sections, 'a1')!
    expect(scope.root.number).toBe('1.1')
    expect(scope.root.children[0]!.number).toBe('1.1.1')
    expect(scope.ancestors.map(section => section.key)).toEqual(['a'])
    expect([...scope.ids].sort()).toEqual(['a1', 'a1x'])
    expect(inFocus(scope, 'a1x')).toBe(true)
    expect(inFocus(scope, 'a2')).toBe(false)
    expect(inFocus(null, 'a2')).toBe(true)
  })
  it('is null for an unknown or empty id', () => {
    expect(sectionFocusScope(sections, 'gone')).toBeNull()
    expect(sectionFocusScope(sections, null)).toBeNull()
  })
  it('counts pending proposals and open threads outside, ignoring unknown sections', () => {
    const scope = sectionFocusScope(sections, 'a')!
    const known = new Set(['a', 'a1', 'a1x', 'a2', 'b'])
    const counts = outsideFocus(scope, known, { a1: 4, b: 2, gone: 5 }, [
      { sectionId: 'b', resolved: false }, { sectionId: 'b', resolved: true }, { sectionId: 'a2', resolved: false }, { sectionId: 'gone', resolved: false },
    ])
    expect(counts).toEqual({ proposals: 2, comments: 1 })
  })
})
