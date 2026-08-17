import { describe, it, expect, vi, afterEach } from 'vitest'
import { renderHook, act, cleanup } from '@testing-library/react'
import { useUndoQueue } from './useUndoQueue'
import type { Question } from '@/lib/questions'

afterEach(cleanup)

function question(id = 'q-1'): Question {
  return {
    id,
    project: 'demo',
    ticket: 'demo-1',
    kind: 'confirm',
    mode: 'blocking',
    status: 'open',
    title: 'Q',
    options: [],
    option_notes: [],
    multi: false,
    recommended_multi: [],
    expertise: [],
    awaiting: 'human',
    created_at: '2026-08-08T00:00:00Z',
  }
}

describe('useUndoQueue visibility', () => {
  it('commits pending answers when the tab is hidden', async () => {
    const commit = vi.fn().mockResolvedValue(undefined)
    const refresh = vi.fn()
    const { result } = renderHook(() =>
      useUndoQueue({ commit, refresh, onError: vi.fn() }),
    )

    act(() => {
      result.current.enqueue(question(), { value: true }, 'Yes', 'detail')
    })

    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      get: () => 'hidden',
    })
    act(() => {
      document.dispatchEvent(new Event('visibilitychange'))
    })

    await vi.waitFor(() => expect(commit).toHaveBeenCalledTimes(1))
    expect(result.current.pending).toHaveLength(0)
    expect(refresh).toHaveBeenCalled()
  })
})
