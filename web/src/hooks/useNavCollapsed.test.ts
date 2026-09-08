import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useNavCollapsed } from './useNavCollapsed'

function stubViewport(width: number) {
  const queries = new Map<string, { matches: boolean; listeners: Set<() => void> }>()
  const state = { width }
  const evaluate = (query: string) => state.width <= Number(/\d+/.exec(query)?.[0] ?? 0)
  vi.stubGlobal('matchMedia', vi.fn((query: string) => {
    const entry = queries.get(query) ?? { matches: evaluate(query), listeners: new Set<() => void>() }
    queries.set(query, entry)
    return {
      get matches() { return entry.matches },
      addEventListener: (_: string, fn: () => void) => entry.listeners.add(fn),
      removeEventListener: (_: string, fn: () => void) => entry.listeners.delete(fn),
    }
  }))
  return {
    resizeTo(next: number) {
      state.width = next
      for (const [query, entry] of queries) {
        const matches = evaluate(query)
        if (matches === entry.matches) continue
        entry.matches = matches
        entry.listeners.forEach(fn => fn())
      }
    },
  }
}

beforeEach(() => localStorage.clear())
afterEach(() => vi.unstubAllGlobals())

describe('useNavCollapsed', () => {
  it('expands on a tablet in one click without touching the desktop preference', () => {
    localStorage.setItem('takomo.nav.collapsed', '1')
    stubViewport(900)
    const { result } = renderHook(() => useNavCollapsed())
    expect(result.current[0]).toBe(true)
    act(() => result.current[1](false))
    expect(result.current[0]).toBe(false)
    expect(localStorage.getItem('takomo.nav.collapsed')).toBe('1')
  })

  it('persists a desktop toggle and re-applies the preference after leaving a narrow width', () => {
    const viewport = stubViewport(1440)
    const { result } = renderHook(() => useNavCollapsed())
    expect(result.current[0]).toBe(false)
    act(() => result.current[1](true))
    expect(result.current[0]).toBe(true)
    expect(localStorage.getItem('takomo.nav.collapsed')).toBe('1')
    act(() => result.current[1](false))
    expect(localStorage.getItem('takomo.nav.collapsed')).toBe('0')
    act(() => viewport.resizeTo(900))
    expect(result.current[0]).toBe(true)
    act(() => viewport.resizeTo(1440))
    expect(result.current[0]).toBe(false)
  })

  it('never persists a phone overlay toggle', () => {
    stubViewport(390)
    const { result } = renderHook(() => useNavCollapsed())
    act(() => result.current[1](false))
    expect(result.current[0]).toBe(false)
    expect(localStorage.getItem('takomo.nav.collapsed')).toBeNull()
  })
})
