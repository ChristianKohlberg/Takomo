// A section without a mounted editor keeps the height it had with one.
//
// The specification mounts editors only near the viewport (`Plan.tsx`). An
// offscreen section shows a short text preview, so without this every editor
// that unmounts behind the reader would shrink its section from the full
// height of its tables and diagrams to a few lines — and every section above a
// navigation target would move the target while the reader travels to it.
//
// Each section's prose sits in a SLOT (the element carrying `min-height`) around
// a CONTENT element (the editor or the preview). The content is measured while
// the editor is mounted — a ResizeObserver on it, so an edit, a table that
// collapses or a diagram that renders updates the record — and when the editor
// goes, the slot keeps that height as its `min-height`. The record is only used
// at the width it was measured at: a narrower pane wraps differently, and a
// wrong floor is worse than the preview's own height.
//
// When an editor mounts again it may take a moment to reach full height (a
// diagram renders asynchronously), so the floor stays until the content has
// grown to it or `graceMs` has passed; only then does the slot follow the
// content, which is what lets a section that really got shorter shrink.
//
// Sections that were never mounted keep the preview; the scroll anchor
// (`scroll-anchor.ts`) absorbs the change when their editor arrives.
//
// Units: the slots sit inside `.document-page`, which carries the reader's CSS
// `zoom` (`lib/document-zoom.ts`). `getBoundingClientRect` there is ZOOMED
// (viewport pixels) while `min-height` and `clientWidth` are the page's own,
// unzoomed pixels. Heights are therefore stored and applied in page pixels —
// the measured rect divided by the zoom — and the whole record is dropped when
// the zoom changes (a preset, a shortcut, or Fit width following the pane),
// because text rewraps at a different page width.

export interface SectionHeightsOptions {
  graceMs?: number
  now?: () => number
}

interface Record { height: number; width: number }

export class SectionHeights {
  private readonly heights = new Map<string, Record>()
  private readonly slots = new Map<string, HTMLElement>()
  private readonly refs = new Map<string, (element: HTMLElement | null) => void>()
  private readonly mountedAt = new Map<string, number>()
  private readonly watched = new Map<string, Element>()
  private readonly owners = new WeakMap<Element, string>()
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>()
  private readonly resize: ResizeObserver | null
  private readonly graceMs: number
  private readonly now: () => number
  private zoom = 1

  constructor({ graceMs = 1000, now = () => performance.now() }: SectionHeightsOptions = {}) {
    this.graceMs = graceMs
    this.now = now
    this.resize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(entries => {
      for (const entry of entries) {
        const id = this.owners.get(entry.target)
        if (id) this.measure(id)
      }
    })
  }

  /** A stable ref callback for a section's slot. */
  slotRef(id: string): (element: HTMLElement | null) => void {
    let ref = this.refs.get(id)
    if (!ref) {
      ref = element => {
        if (element) this.slots.set(id, element)
        else if (this.slots.has(id)) { this.slots.delete(id); this.release(id) }
      }
      this.refs.set(id, ref)
    }
    return ref
  }

  /**
   * The document zoom in effect. Call before `sync` on every render; a change
   * forgets every recorded height, so the next `sync` of an unmounted section
   * clears its floor instead of applying one measured at another zoom.
   */
  setZoom(zoom: number): void {
    const next = zoom > 0 && Number.isFinite(zoom) ? zoom : 1
    if (Math.abs(next - this.zoom) < 1e-6) return
    this.zoom = next
    this.heights.clear()
  }

  /** The last recorded height in page (unzoomed) pixels, if any. */
  heightOf(id: string): number | undefined {
    return this.heights.get(id)?.height
  }

  /** After each render: is this section's editor mounted? */
  sync(id: string, editor: boolean): void {
    const slot = this.slots.get(id)
    if (!slot) return
    if (!editor) {
      this.release(id)
      const record = this.heights.get(id)
      const floor = record && Math.abs(record.width - slot.clientWidth) < 1 ? `${record.height}px` : ''
      if (slot.style.minHeight !== floor) slot.style.minHeight = floor
      return
    }
    const content = slot.firstElementChild
    if (content && this.watched.get(id) !== content) {
      const previous = this.watched.get(id)
      if (previous) this.resize?.unobserve(previous)
      this.watched.set(id, content)
      this.owners.set(content, id)
      this.resize?.observe(content)
    }
    if (!this.mountedAt.has(id)) {
      this.mountedAt.set(id, this.now())
      if (slot.style.minHeight) this.timers.set(id, setTimeout(() => { this.timers.delete(id); this.measure(id, true) }, this.graceMs))
    }
    this.measure(id)
  }

  /** Record the mounted editor's height; let the floor go once it is no longer needed. */
  measure(id: string, expired = false): void {
    const slot = this.slots.get(id)
    const content = slot?.firstElementChild
    const since = this.mountedAt.get(id)
    if (!slot || !content || since === undefined) return
    // Page pixels: the rect is zoomed, `min-height` is not.
    const height = content.getBoundingClientRect().height / this.zoom
    const floor = parseFloat(slot.style.minHeight) || 0
    if (floor > 0) {
      const waiting = !expired && this.now() - since < this.graceMs
      if (height < floor - 1 && waiting) return
      slot.style.minHeight = ''
      clearTimeout(this.timers.get(id))
      this.timers.delete(id)
    }
    if (height > 0) this.heights.set(id, { height, width: slot.clientWidth })
  }

  private release(id: string) {
    this.mountedAt.delete(id)
    clearTimeout(this.timers.get(id))
    this.timers.delete(id)
    const watched = this.watched.get(id)
    if (watched) this.resize?.unobserve(watched)
    this.watched.delete(id)
  }

  /** Stops observing; the next `sync` starts again (React may re-run effects). */
  dispose(): void {
    this.resize?.disconnect()
    for (const timer of this.timers.values()) clearTimeout(timer)
    this.timers.clear()
    this.watched.clear()
    this.mountedAt.clear()
  }
}
