import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

import { between, first, isValid, sequence } from './fracdex'

// The same file `src/fracdex.rs` reads. Two implementations, one contract.
const vectors = JSON.parse(
  // Resolved from the vitest root (`web/`), not from this file: the transform
  // rewrites `import.meta.url` and it is no longer a file URL by the time it runs.
  readFileSync(resolve(process.cwd(), '../tests/fixtures/fracdex-vectors.json'), 'utf8'),
) as {
  between: { a: string | null; b: string | null; want: string }[]
  valid: string[]
  invalid: string[]
}

describe('fracdex', () => {
  it('matches the shared vectors', () => {
    for (const { a, b, want } of vectors.between) {
      expect(between(a, b), `between(${JSON.stringify(a)}, ${JSON.stringify(b)})`).toBe(want)
    }
  })

  it('accepts every key the vectors call valid', () => {
    for (const key of vectors.valid) expect(isValid(key), key).toBe(true)
  })

  it('refuses every key the vectors call invalid', () => {
    for (const key of vectors.invalid) expect(isValid(key), key).toBe(false)
  })

  it('lands a new key exactly where it was aimed', () => {
    const ring = [first()]
    for (let round = 0; round < 40; round += 1) {
      const at = (round * 7) % (ring.length + 1)
      const lo = at === 0 ? null : (ring[at - 1] as string)
      const hi = at < ring.length ? (ring[at] as string) : null
      const key = between(lo, hi)

      expect(isValid(key)).toBe(true)
      if (lo !== null) expect(lo < key).toBe(true)
      if (hi !== null) expect(key < hi).toBe(true)
      ring.splice(at, 0, key)
    }
    expect(ring).toEqual([...ring].sort())
  })

  it('stays bounded when one gap is split over and over', () => {
    const lo = first()
    let hi = between(lo, null)
    for (let i = 0; i < 64; i += 1) {
      const mid = between(lo, hi)
      expect(lo < mid && mid < hi).toBe(true)
      hi = mid
    }
    expect(hi.length).toBeLessThanOrEqual(40)
  })

  it('appends rather than throwing when the pair arrives out of order', () => {
    // Two peers can each hold a state the other has not seen.
    const key = between('k', 'V')
    expect(isValid(key)).toBe(true)
    expect(key > 'k').toBe(true)
  })

  it('seeds an ascending ring', () => {
    const keys = sequence(200)
    expect(keys).toHaveLength(200)
    expect(keys).toEqual([...keys].sort())
    expect(keys.every(isValid)).toBe(true)
  })
})
