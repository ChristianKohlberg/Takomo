// Browser APIs jsdom does not implement, stubbed so Radix's overlay primitives
// can be driven in a test at all.
//
// This is not a convenience. jsdom has no layout engine, and Radix's Select,
// DropdownMenu and Popover all measure and capture the pointer before they will
// open — so without these the control simply never opens, and a test that tries
// to choose an option fails in a way that looks like the component is broken
// rather than like the environment is missing a method.
//
// It buys back the coverage the move off `<select>` would otherwise have cost:
// `fireEvent.change` drives a native select directly, and there is no equivalent
// for a listbox that has to open first. Deleting a filtering test because the
// widget changed would quietly stop checking that filtering works.
//
// Each stub is the minimum the primitives call, deliberately — a fuller fake
// layout engine would start asserting things about geometry that jsdom cannot
// honour anyway.

// Radix sets pointer capture on the trigger while a press is in flight.
if (!Element.prototype.hasPointerCapture) {
  Element.prototype.hasPointerCapture = () => false
  Element.prototype.setPointerCapture = () => {}
  Element.prototype.releasePointerCapture = () => {}
}

// The selected item is scrolled into view when the listbox opens.
if (!Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = () => {}
}

// Popper measures its anchor and content to decide which way to open.
if (typeof globalThis.ResizeObserver === 'undefined') {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver
}

if (typeof globalThis.DOMRect === 'undefined') {
  globalThis.DOMRect = class {
    constructor(
      public x = 0,
      public y = 0,
      public width = 0,
      public height = 0,
    ) {}
    get top() {
      return this.y
    }
    get left() {
      return this.x
    }
    get right() {
      return this.x + this.width
    }
    get bottom() {
      return this.y + this.height
    }
    static fromRect(r?: DOMRectInit) {
      return new DOMRect(r?.x, r?.y, r?.width, r?.height)
    }
    toJSON() {
      return { ...this }
    }
  } as unknown as typeof DOMRect
}
