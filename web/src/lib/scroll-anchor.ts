// Getting to a place in a long document and STAYING there while it settles.
//
// A section jump in the specification cannot be a single `scrollIntoView`: the
// destination is computed once, and then the layout above it keeps moving —
// editors mount near the viewport and grow to full height (tables, diagrams,
// the long-table collapse), editors that leave unmount and shrink back to their
// preview. A smooth scroll aims at a point that is no longer where the section
// is by the time it arrives; an instant one is right for one frame.
//
// So a jump is instant, and then ANCHORED: every frame the target's position is
// measured again and the container's `scrollTop` corrected until the layout has
// held still for `settleMs`, or `capMs` has passed, or the reader takes over
// (wheel, touch, a press in the column — the scrollbar included — or any key).
// Nothing animates, so `prefers-reduced-motion` has nothing to reduce.
//
// Browser scroll anchoring (`overflow-anchor`) solves the neighbouring problem —
// keep what is on screen still while content above changes — with its own
// choice of anchor node. While this helper is correcting it is switched off on
// the container, so exactly one mechanism decides where the column is; it is
// restored when the anchor stops, and from then on the browser keeps the
// reader's place through any late change (a diagram that renders after 2 s).
//
// One anchor per container: starting a new one cancels the previous one.

export type AnchorAlign = 'start' | 'center'
export type AnchorStop = 'stable' | 'cap' | 'user' | 'cancelled' | 'missing'

export interface AnchorOptions {
  /** The element to bring into place, or a lookup — it may mount only after the jump starts. */
  target: HTMLElement | null | (() => HTMLElement | null | undefined)
  /** The scrolling element. Default: the target's nearest scrollable ancestor. */
  container?: HTMLElement | null
  /** `start` puts the target's top at the container's top; `center` centres it. */
  align?: AnchorAlign
  /** Pixels between the container's top edge and the target (`start` only).
   *  Container pixels: the document's reader zoom does not scale it. Positions
   *  themselves are rect differences, which are zoom-correct as they are. */
  offset?: number
  /** How long the layout must hold still before the anchor lets go. */
  settleMs?: number
  /** The anchor never holds on longer than this. */
  capMs?: number
  onStop?: (reason: AnchorStop) => void
  /** Test seams: time and frame scheduling. */
  now?: () => number
  frame?: (callback: () => void) => number
  cancelFrame?: (handle: number) => void
}

export interface ScrollAnchor {
  cancel(): void
  readonly stopped: AnchorStop | null
}

export const ANCHOR_SETTLE_MS = 300
export const ANCHOR_CAP_MS = 2000

const active = new WeakMap<HTMLElement, ScrollAnchor>()

/** Stop whatever anchor holds `container`, e.g. before setting `scrollTop` by hand. */
export function cancelScrollAnchor(container: HTMLElement | null | undefined): void {
  if (container) active.get(container)?.cancel()
}

/** The nearest ancestor that scrolls vertically. */
export function scrollContainerOf(element: HTMLElement): HTMLElement | null {
  for (let node = element.parentElement; node; node = node.parentElement) {
    const overflow = getComputedStyle(node).overflowY
    if (overflow === 'auto' || overflow === 'scroll') return node
  }
  return (document.scrollingElement as HTMLElement | null) ?? null
}

/** Where `container.scrollTop` must be for `element` to sit in place, clamped to what can scroll. */
export function anchoredScrollTop(container: HTMLElement, element: HTMLElement, align: AnchorAlign = 'start', offset = 0): number {
  const box = container.getBoundingClientRect()
  const rect = element.getBoundingClientRect()
  const delta = align === 'center'
    ? rect.top + rect.height / 2 - (box.top + container.clientTop + container.clientHeight / 2)
    : rect.top - (box.top + container.clientTop) - offset
  const max = Math.max(0, container.scrollHeight - container.clientHeight)
  return Math.min(max, Math.max(0, container.scrollTop + delta))
}

// Input that means the reader has taken the scroll position over. `pointerdown`
// on the container covers the scrollbar, which is part of its box.
const CONTAINER_INPUT = ['wheel', 'touchstart', 'pointerdown'] as const

/** Jump to `target` now and keep it there until the layout settles. */
export function anchorScroll(options: AnchorOptions): ScrollAnchor {
  const {
    align = 'start',
    offset = 0,
    settleMs = ANCHOR_SETTLE_MS,
    capMs = ANCHOR_CAP_MS,
    now = () => performance.now(),
    frame = callback => requestAnimationFrame(callback),
    cancelFrame = handle => cancelAnimationFrame(handle),
  } = options
  const resolve = typeof options.target === 'function' ? options.target : () => options.target as HTMLElement | null
  const started = now()
  let lastChange = started
  let container: HTMLElement | null = null
  let savedAnchor = ''
  let handle: number | null = null
  let resize: ResizeObserver | null = null
  let observed: Element | null = null
  let stopped: AnchorStop | null = null

  const onUser = () => stop('user')
  const onKey = (event: KeyboardEvent) => {
    // Modifier presses on their own are not navigation.
    if (event.key === 'Shift' || event.key === 'Control' || event.key === 'Alt' || event.key === 'Meta') return
    stop('user')
  }

  const anchor: ScrollAnchor = {
    cancel: () => stop('cancelled'),
    get stopped() { return stopped },
  }

  function attach(scroller: HTMLElement) {
    container = scroller
    active.get(scroller)?.cancel()
    active.set(scroller, anchor)
    savedAnchor = scroller.style.overflowAnchor
    scroller.style.overflowAnchor = 'none'
    for (const type of CONTAINER_INPUT) scroller.addEventListener(type, onUser, { passive: true })
    window.addEventListener('keydown', onKey, true)
    if (typeof ResizeObserver !== 'undefined') {
      // Any size change in the content counts as the layout still moving, even
      // one that did not (yet) shift the target: it is what arrives next frame.
      resize = new ResizeObserver(() => { lastChange = now() })
      for (const child of Array.from(scroller.children)) resize.observe(child)
    }
  }

  function stop(reason: AnchorStop) {
    if (stopped) return
    stopped = reason
    if (handle !== null) cancelFrame(handle)
    handle = null
    resize?.disconnect()
    if (container) {
      for (const type of CONTAINER_INPUT) container.removeEventListener(type, onUser)
      container.style.overflowAnchor = savedAnchor
      if (active.get(container) === anchor) active.delete(container)
    }
    window.removeEventListener('keydown', onKey, true)
    options.onStop?.(reason)
  }

  function step() {
    handle = null
    if (stopped) return
    const time = now()
    const element = resolve()
    if (!element || !element.isConnected) {
      if (time - started >= capMs) stop('missing')
      else handle = frame(step)
      return
    }
    if (!container) {
      const scroller = options.container ?? scrollContainerOf(element)
      if (!scroller) return stop('missing')
      attach(scroller)
    }
    const scroller = container!
    if (resize && observed !== element) {
      if (observed) resize.unobserve(observed)
      resize.observe(element)
      observed = element
    }
    const want = anchoredScrollTop(scroller, element, align, offset)
    if (Math.abs(want - scroller.scrollTop) >= 1) {
      const before = scroller.scrollTop
      scroller.scrollTop = want
      if (Math.abs(scroller.scrollTop - before) >= 1) lastChange = time
    }
    if (time - lastChange >= settleMs) stop('stable')
    else if (time - started >= capMs) stop('cap')
    else handle = frame(step)
  }

  if (options.container) attach(options.container)
  step()
  return anchor
}

/** An editor position (a comment's text, a search passage), centred and anchored. */
export function revealEditorPosition(editor: { view: { domAtPos(position: number): { node: Node } } }, position: number): ScrollAnchor | null {
  const { node } = editor.view.domAtPos(position)
  const element = node instanceof HTMLElement ? node : node.parentElement
  return element ? anchorScroll({ target: element, align: 'center' }) : null
}
