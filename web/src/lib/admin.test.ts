import { describe, expect, it } from 'vitest'
import { formatBytes, projectAllowlist } from './admin'

describe('projectAllowlist', () => {
  // The regression this function exists for. `/v1/whoami` reports an
  // unrestricted token as the string '*', which is truthy AND has a length, so
  // the obvious guard treated the least scoped token in the system as though it
  // were fenced to a single project — and the render path called `.join()` on a
  // string and threw.
  it("treats '*' as unrestricted, not as a one-project allowlist", () => {
    expect(projectAllowlist({ projects: '*' })).toBeNull()
  })

  it('returns the allowlist when there is one', () => {
    expect(projectAllowlist({ projects: ['demo', 'takomo'] })).toEqual(['demo', 'takomo'])
  })

  it('treats a missing field and an empty list as unrestricted', () => {
    expect(projectAllowlist({})).toBeNull()
    expect(projectAllowlist(null)).toBeNull()
    expect(projectAllowlist({ projects: [] })).toBeNull()
  })
})

describe('formatBytes', () => {
  it('keeps small sizes in bytes', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(1023)).toBe('1023 B')
  })

  it('steps up through the units', () => {
    expect(formatBytes(1024)).toBe('1.0 KB')
    expect(formatBytes(417792)).toBe('408 KB')
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB')
    expect(formatBytes(3 * 1024 * 1024 * 1024)).toBe('3.0 GB')
  })
})
