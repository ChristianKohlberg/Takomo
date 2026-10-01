import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SectionHeights } from './section-heights'

const observers: FakeResizeObserver[] = []
class FakeResizeObserver {
  observed = new Set<Element>()
  constructor(public callback: ResizeObserverCallback) { observers.push(this) }
  observe(element: Element) { this.observed.add(element) }
  unobserve(element: Element) { this.observed.delete(element) }
  disconnect() { this.observed.clear() }
  resize(element: Element) { this.callback([{ target: element } as ResizeObserverEntry], this as unknown as ResizeObserver) }
}

/** A slot whose content reports `height`, at a column width of `width`. */
function slot(keeper: SectionHeights, id: string) {
  const element = document.createElement('div')
  const content = document.createElement('div')
  element.append(content)
  const state = { height: 1200, width: 700 }
  content.getBoundingClientRect = () => new DOMRect(0, 0, state.width, state.height)
  Object.defineProperty(element, 'clientWidth', { get: () => state.width })
  keeper.slotRef(id)(element)
  return { element, content, state }
}

let time = 0
beforeEach(() => {
  time = 0
  observers.length = 0
  vi.useFakeTimers()
  vi.stubGlobal('ResizeObserver', FakeResizeObserver)
})
afterEach(() => {
  vi.useRealTimers()
  vi.unstubAllGlobals()
})

describe('SectionHeights', () => {
  it('keeps the mounted height as the preview\'s min-height once the editor unmounts', () => {
    const keeper = new SectionHeights({ now: () => time })
    const { element, content, state } = slot(keeper, 'a')
    keeper.sync('a', true)
    expect(element.style.minHeight).toBe('')
    // The editor grows (a table renders); the record follows it.
    state.height = 1800
    observers[0]!.resize(content)
    expect(keeper.heightOf('a')).toBe(1800)
    keeper.sync('a', false)
    expect(element.style.minHeight).toBe('1800px')
  })

  it('leaves a never-mounted section at its preview height', () => {
    const keeper = new SectionHeights({ now: () => time })
    const { element } = slot(keeper, 'a')
    keeper.sync('a', false)
    expect(element.style.minHeight).toBe('')
  })

  it('does not apply a height measured at another width', () => {
    const keeper = new SectionHeights({ now: () => time })
    const { element, state } = slot(keeper, 'a')
    keeper.sync('a', true)
    state.width = 400
    keeper.sync('a', false)
    expect(element.style.minHeight).toBe('')
  })

  it('holds the floor while a remounted editor is still growing, then follows the content', () => {
    const keeper = new SectionHeights({ now: () => time, graceMs: 1000 })
    const { element, content, state } = slot(keeper, 'a')
    keeper.sync('a', true)
    keeper.sync('a', false)
    expect(element.style.minHeight).toBe('1200px')
    // Remounted: the editor starts short, the floor stays.
    state.height = 300
    keeper.sync('a', true)
    expect(element.style.minHeight).toBe('1200px')
    // It reaches full height: the floor goes, the slot follows the content again.
    state.height = 1250
    observers[0]!.resize(content)
    expect(element.style.minHeight).toBe('')
    expect(keeper.heightOf('a')).toBe(1250)
  })

  it('lets a section that really got shorter shrink after the grace period', () => {
    const keeper = new SectionHeights({ now: () => time, graceMs: 1000 })
    const { element, state } = slot(keeper, 'a')
    keeper.sync('a', true)
    keeper.sync('a', false)
    state.height = 500
    keeper.sync('a', true)
    expect(element.style.minHeight).toBe('1200px')
    time += 1000
    vi.advanceTimersByTime(1000)
    expect(element.style.minHeight).toBe('')
    expect(keeper.heightOf('a')).toBe(500)
  })

  it('forgets the slot element, not the height, when a section leaves the page', () => {
    const keeper = new SectionHeights({ now: () => time })
    slot(keeper, 'a')
    keeper.sync('a', true)
    keeper.slotRef('a')(null)
    expect(keeper.heightOf('a')).toBe(1200)
    // Rendered again (a section focus was left): the floor applies to the new slot.
    const again = slot(keeper, 'a')
    keeper.sync('a', false)
    expect(again.element.style.minHeight).toBe('1200px')
  })
})
