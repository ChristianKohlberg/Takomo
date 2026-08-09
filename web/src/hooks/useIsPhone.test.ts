// The one place the phone breakpoint exists in JS rather than CSS.
//
// It is worth a test precisely because it duplicates a number Tailwind already
// knows: if these drift apart, the board mounts the wrong number of columns for
// the layout the stylesheet is producing, and nothing else would say so.
import { describe, it, expect, vi, afterEach } from 'vitest'
import { renderHook, act } from '@testing-library/react'
import { useIsPhone, PHONE_MAX } from './useIsPhone'

/** A controllable `matchMedia` — jsdom's always reports `matches: false`. */
function stubMedia(matches: boolean) {
  const listeners = new Set<() => void>()
  const mq = {
    matches,
    addEventListener: (_: string, fn: () => void) => listeners.add(fn),
    removeEventListener: (_: string, fn: () => void) => listeners.delete(fn),
  }
  vi.stubGlobal(
    'matchMedia',
    vi.fn(() => mq),
  )
  return {
    mq,
    resizeTo(next: boolean) {
      mq.matches = next
      listeners.forEach((fn) => fn())
    },
  }
}

afterEach(() => vi.unstubAllGlobals())

describe('useIsPhone', () => {
  it('agrees with Tailwind: md is 768, so phone is <= 767', () => {
    // If Tailwind's `md` ever moves, this number has to move with it — the
    // board would otherwise render one column while the CSS lays out three.
    expect(PHONE_MAX).toBe(767)
  })

  it('reports a phone viewport', () => {
    stubMedia(true)
    expect(renderHook(() => useIsPhone()).result.current).toBe(true)
  })

  it('reports a desktop viewport', () => {
    stubMedia(false)
    expect(renderHook(() => useIsPhone()).result.current).toBe(false)
  })

  it('follows a rotation rather than reading once at mount', () => {
    // Turning a phone sideways crosses this line; a value read once would leave
    // the board showing a single column on a landscape tablet.
    const media = stubMedia(true)
    const { result } = renderHook(() => useIsPhone())
    expect(result.current).toBe(true)
    act(() => media.resizeTo(false))
    expect(result.current).toBe(false)
  })

  it('survives an environment with no matchMedia', () => {
    vi.stubGlobal('matchMedia', undefined)
    expect(renderHook(() => useIsPhone()).result.current).toBe(false)
  })
})
