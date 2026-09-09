import { describe, expect, it } from 'vitest'
import { embeddingState } from './embedding-status'
import type { SearchStatus } from './hybrid-search'
export const current: SearchStatus = { configured: true, indexed: 2, total: 2, passages_indexed: 7, passages_total: 7, queued: 0, pending: 0, running: 0, failed: 0, last_error: null, projection: 'current', last_synced_at: 1750000000000 }
describe('embedding completion', () => {
  it('requires passage coverage rather than matching section counts', () => {
    expect(embeddingState(current, '', false, false)).toBe('current')
    expect(embeddingState({ ...current, passages_indexed: 6 }, '', false, false)).toBe('stale')
    expect(embeddingState({ ...current, passages_indexed: 0, passages_total: 0, last_synced_at: null }, '', false, false)).toBe('current')
  })
  it.each([
    [{ configured: false }, 'unconfigured'], [{ pending: 1, queued: 1 }, 'pending'], [{ running: 1, queued: 1 }, 'running'],
    [{ failed: 1, queued: 1 }, 'error'], [{ last_error: 'retry failed' }, 'error'], [{ projection: 'stale' }, 'stale'],
  ] as const)('rejects current for %j', (patch, expected) => {
    expect(embeddingState({ ...current, ...patch }, '', false, false)).toBe(expected)
  })
  it('does not show completion while edits are local, reads are stale, or requests failed', () => {
    expect(embeddingState(current, '', true, false)).toBe('pending')
    expect(embeddingState(current, '', false, true)).toBe('loading')
    expect(embeddingState(current, 'offline', false, false)).toBe('error')
  })
})
