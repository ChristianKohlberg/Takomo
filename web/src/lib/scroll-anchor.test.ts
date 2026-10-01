import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { anchorScroll, cancelScrollAnchor, type AnchorStop } from './scroll-anchor'

// jsdom has no layout, so the test owns one: a column 800 px tall whose content
// is `height` px, and a target whose top sits `top` px into that content. Frames
// and time are driven by hand.
function layout() {
  const column = document.createElement('div')
  const content = document.createElement('div')
  const target = document.createElement('section')
  column.append(content)
  content.append(target)
  document.body.append(column)
  const state = { scrollTop: 0, top: 5000, height: 20000, writes: 0 }
  Object.defineProperty(column, 'scrollTop', {
    get: () => state.scrollTop,
    set: (value: number) => { state.writes++; state.scrollTop = Math.max(0, Math.min(value, state.height - 800)) },
  })
  Object.defineProperty(column, 'scrollHeight', { get: () => state.height })
  Object.defineProperty(column, 'clientHeight', { get: () => 800 })
  column.getBoundingClientRect = () => new DOMRect(0, 100, 600, 800)
  target.getBoundingClientRect = () => new DOMRect(0, 100 + state.top - state.scrollTop, 600, 300)
  return { column, content, target, state }
}

let time = 0
let frames: (() => void)[] = []
const clock = {
  now: () => time,
  frame: (callback: () => void) => { frames.push(callback); return frames.length },
  cancelFrame: () => { frames = [] },
}
/** Advance one 16 ms frame. */
function tick(times = 1) {
  for (let i = 0; i < times; i++) {
    time += 16
    const due = frames
    frames = []
    for (const callback of due) callback()
  }
}

const observers: FakeResizeObserver[] = []
class FakeResizeObserver {
  observed = new Set<Element>()
  constructor(public callback: ResizeObserverCallback) { observers.push(this) }
  observe(element: Element) { this.observed.add(element) }
  unobserve(element: Element) { this.observed.delete(element) }
  disconnect() { this.observed.clear() }
  fire() { this.callback([], this as unknown as ResizeObserver) }
}

beforeEach(() => {
  time = 0
  frames = []
  observers.length = 0
  vi.stubGlobal('ResizeObserver', FakeResizeObserver)
})
afterEach(() => {
  vi.unstubAllGlobals()
  document.body.replaceChildren()
})

describe('anchorScroll', () => {
  it('jumps at once, without waiting for a frame', () => {
    const { column, target, state } = layout()
    anchorScroll({ container: column, target, ...clock })
    expect(state.scrollTop).toBe(5000)
  })

  it('corrects every frame while the layout above the target changes, then stops once stable', () => {
    const { column, target, state } = layout()
    const stops: AnchorStop[] = []
    anchorScroll({ container: column, target, onStop: reason => stops.push(reason), ...clock })
    // Editors above the target mount and grow over a few frames.
    for (const grown of [400, 900, 1600]) {
      state.top += grown
      tick()
      expect(state.scrollTop).toBe(state.top)
    }
    expect(stops).toEqual([])
    tick(Math.ceil(300 / 16) + 1)
    expect(stops).toEqual(['stable'])
    // Once stopped, a change is no longer corrected (native anchoring's job now).
    state.top += 500
    tick(3)
    expect(state.scrollTop).toBe(state.top - 500)
  })

  it('counts a resize anywhere in the content as the layout still moving', () => {
    const { column, content, target } = layout()
    const stops: AnchorStop[] = []
    anchorScroll({ container: column, target, onStop: reason => stops.push(reason), ...clock })
    expect(observers[0]?.observed.has(content)).toBe(true)
    expect(observers[0]?.observed.has(target)).toBe(true)
    for (let i = 0; i < 30; i++) { observers[0]!.fire(); tick() }
    expect(stops).toEqual([])
    tick(Math.ceil(300 / 16) + 1)
    expect(stops).toEqual(['stable'])
  })

  it('gives up at the cap when the layout never settles', () => {
    const { column, target, state } = layout()
    const stops: AnchorStop[] = []
    anchorScroll({ container: column, target, capMs: 2000, onStop: reason => stops.push(reason), ...clock })
    for (let i = 0; i < 200 && stops.length === 0; i++) { state.top += 10; tick() }
    expect(stops).toEqual(['cap'])
    expect(time).toBeGreaterThanOrEqual(2000)
    expect(time).toBeLessThan(2100)
  })

  it.each([
    ['wheel', (column: HTMLElement) => column.dispatchEvent(new Event('wheel'))],
    ['touch', (column: HTMLElement) => column.dispatchEvent(new Event('touchstart'))],
    ['pointer (the scrollbar)', (column: HTMLElement) => column.dispatchEvent(new Event('pointerdown'))],
    ['key', () => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'PageDown' }))],
  ])('lets go immediately when the reader scrolls: %s', (_, act) => {
    const { column, target, state } = layout()
    column.style.overflowAnchor = 'auto'
    const stops: AnchorStop[] = []
    anchorScroll({ container: column, target, onStop: reason => stops.push(reason), ...clock })
    expect(column.style.overflowAnchor).toBe('none')
    act(column)
    expect(stops).toEqual(['user'])
    expect(column.style.overflowAnchor).toBe('auto')
    const writes = state.writes
    state.top += 700
    tick(5)
    expect(state.writes).toBe(writes)
  })

  it('ignores a modifier key on its own', () => {
    const { column, target } = layout()
    const stops: AnchorStop[] = []
    anchorScroll({ container: column, target, onStop: reason => stops.push(reason), ...clock })
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Shift' }))
    expect(stops).toEqual([])
  })

  it('waits for a target that mounts after the jump starts', () => {
    const { column, target, state } = layout()
    let mounted = false
    anchorScroll({ container: column, target: () => (mounted ? target : null), ...clock })
    tick(3)
    expect(state.scrollTop).toBe(0)
    mounted = true
    tick()
    expect(state.scrollTop).toBe(5000)
  })

  it('centres when asked, and honours an offset at the start', () => {
    const { column, target, state } = layout()
    anchorScroll({ container: column, target, align: 'center', ...clock })
    // Target centre (top + 150) at the column centre (400).
    expect(state.scrollTop).toBe(5000 + 150 - 400)
    anchorScroll({ container: column, target, offset: 40, ...clock })
    expect(state.scrollTop).toBe(5000 - 40)
  })

  it('replaces the previous anchor on the same column, and can be cancelled', () => {
    const { column, target, state } = layout()
    const first: AnchorStop[] = []
    const second: AnchorStop[] = []
    anchorScroll({ container: column, target, onStop: reason => first.push(reason), ...clock })
    anchorScroll({ container: column, target, onStop: reason => second.push(reason), ...clock })
    expect(first).toEqual(['cancelled'])
    cancelScrollAnchor(column)
    expect(second).toEqual(['cancelled'])
    state.top += 100
    tick(2)
    expect(state.scrollTop).toBe(5000)
  })
})
