import { describe, expect, it } from 'vitest'
import { isValidProjectId } from './NewProjectDialog'

// The id prefixes every ticket in the project and cannot be changed afterwards,
// so this runs as the field is typed rather than on submit — a rejected id that
// only reveals itself after pressing Create is one the person has already
// written into a config somewhere.
describe('isValidProjectId', () => {
  it('accepts lowercase, digits and dashes', () => {
    expect(isValidProjectId('demo')).toBe(true)
    expect(isValidProjectId('takomo')).toBe(true)
    expect(isValidProjectId('web-2')).toBe(true)
    expect(isValidProjectId('9lives')).toBe(true)
  })

  it('rejects an empty id', () => {
    expect(isValidProjectId('')).toBe(false)
  })

  it('rejects uppercase and whitespace', () => {
    expect(isValidProjectId('Demo')).toBe(false)
    expect(isValidProjectId('my project')).toBe(false)
    expect(isValidProjectId('demo ')).toBe(false)
  })

  it('rejects a leading dash and characters that would break a ticket id', () => {
    expect(isValidProjectId('-demo')).toBe(false)
    expect(isValidProjectId('demo/prod')).toBe(false)
    expect(isValidProjectId('demo_prod')).toBe(false)
    expect(isValidProjectId('demo.prod')).toBe(false)
  })
})
