// Zoom for the specification's Document view: a personal, per-browser reading
// preference, like Word's or Google Docs' zoom. It scales the document COLUMN
// only (`.document-page`, via CSS `zoom`) and never the app chrome, the outline,
// the toolbar or dialogs. Nothing here is written to the CRDT or the server.
//
// Why CSS `zoom` and not `transform: scale` or a root font size: see
// `docs/documents.md` ("Zoom"). The short version: `zoom` reflows (the page
// keeps fitting its column and the scroll height is the real one), and since
// the standardisation (Chromium 128, Firefox 126, Safari) `getBoundingClientRect`,
// `Range.getClientRects`, `caretPositionFromPoint` and IntersectionObserver all
// report the zoomed, viewport-space geometry ProseMirror expects. The one place
// that mixes spaces is prosemirror-tables' column-resize drag, which adds a
// viewport `clientX` delta to a CSS-pixel width; `correctZoomedResizeDrag`
// below scales that delta back.

export const ZOOM_PRESETS = [0.5, 0.75, 0.9, 1, 1.25, 1.5, 2] as const
export const DEFAULT_ZOOM = 1
export const MIN_ZOOM = ZOOM_PRESETS[0]
export const MAX_ZOOM = ZOOM_PRESETS[ZOOM_PRESETS.length - 1]!
/** `.document-page`'s max-width in `styles/editor.css`: the measure "fit width" fills. */
export const PAGE_MEASURE = 840

/** A preset factor, or "fit width", which is recomputed as the pane changes. */
export type ZoomSetting = number | 'fit'

const keyFor = (project: string) => `takomo:document-zoom:${project}`

export function readZoom(project: string): ZoomSetting {
  try {
    return parseZoom(localStorage.getItem(keyFor(project)))
  } catch {
    return DEFAULT_ZOOM
  }
}

/** Only a preset or `fit` survives a round trip; anything else is the default. */
export function parseZoom(raw: string | null): ZoomSetting {
  if (raw === 'fit') return 'fit'
  const value = Number(raw)
  return (ZOOM_PRESETS as readonly number[]).includes(value) ? value : DEFAULT_ZOOM
}

export function writeZoom(project: string, setting: ZoomSetting): void {
  try {
    if (setting === DEFAULT_ZOOM) localStorage.removeItem(keyFor(project))
    else localStorage.setItem(keyFor(project), String(setting))
  } catch {
    // A private window refuses storage; the zoom still works for this visit.
  }
}

/**
 * The factor that makes the page's measure fill `available` CSS pixels.
 *
 * Never below 100 %: the page is a max-width measure, not a fixed sheet, so in a
 * pane narrower than the measure it already reflows to fit, and shrinking the
 * text there would only make it smaller. Never above the largest preset.
 */
export function fitZoom(available: number, measure = PAGE_MEASURE): number {
  if (!(available > 0) || !(measure > 0)) return DEFAULT_ZOOM
  const exact = available / measure
  return Math.min(MAX_ZOOM, Math.max(DEFAULT_ZOOM, Math.floor(exact * 100) / 100))
}

/** The next preset in `direction` from the zoom now in effect (fit included). */
export function stepZoom(current: number, direction: 1 | -1): number {
  const presets = ZOOM_PRESETS as readonly number[]
  if (direction > 0) return presets.find(value => value > current + 0.001) ?? MAX_ZOOM
  return [...presets].reverse().find(value => value < current - 0.001) ?? MIN_ZOOM
}

export function formatZoom(value: number, locale: 'en' | 'de'): string {
  const percent = Math.round(value * 100)
  // German typography separates the sign; a no-break space keeps it on the line.
  return locale === 'de' ? `${percent}\u00a0%` : `${percent}%`
}

export type ZoomCommand = 'in' | 'out' | 'reset'

/**
 * Ctrl+Alt+Plus / Minus / 0 (⌘+Option on macOS). Plain Ctrl/⌘ +/−/0 stay the
 * browser's own page zoom and are never claimed.
 *
 * With Ctrl+Alt the KEY is read, not the physical code: on Windows Ctrl+Alt is
 * AltGr, and AltGr+0 is `}` on a German layout — a character a writer types,
 * which must not reset the zoom. The keypad has no AltGr characters, so its
 * codes are accepted too (German Windows: AltGr+`+` is `~`, so the keypad plus
 * is the way in there). With ⌘+Option the code is read, because Option changes
 * the key (⌘+Option+= reports `≠`) and ⌘ means no character is being typed.
 */
export function zoomShortcut(event: Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'isComposing'>): ZoomCommand | null {
  if (!event.altKey || event.isComposing) return null
  const code = event.code
  if (event.metaKey && !event.ctrlKey) {
    if (code === 'Equal' || code === 'NumpadAdd') return 'in'
    if (code === 'Minus' || code === 'NumpadSubtract') return 'out'
    if (code === 'Digit0' || code === 'Numpad0') return 'reset'
    return null
  }
  if (!event.ctrlKey || event.metaKey) return null
  if (event.key === '+' || event.key === '=' || code === 'NumpadAdd') return 'in'
  if (event.key === '-' || code === 'NumpadSubtract') return 'out'
  if (event.key === '0' || code === 'Numpad0') return 'reset'
  return null
}

/**
 * Makes prosemirror-tables' column-resize drag follow the pointer under zoom.
 *
 * The plugin computes `startWidth + (clientX - startX)`: a width in the table's
 * own CSS pixels plus a pointer delta in viewport pixels. Under `zoom: 2` the
 * column would grow twice as fast as the pointer moves. While a drag that
 * started on a resize handle inside `root` is in progress, each mouse event's
 * `clientX` is re-expressed as `startX + delta / zoom` before the plugin's
 * window listeners read it. Returns the cleanup.
 */
export function correctZoomedResizeDrag(root: HTMLElement, zoom: () => number): () => void {
  const win = root.ownerDocument.defaultView ?? window
  let drag: { startX: number; zoom: number } | null = null
  const rewrite = (event: MouseEvent) => {
    if (!drag) return
    const x = drag.startX + (event.clientX - drag.startX) / drag.zoom
    Object.defineProperty(event, 'clientX', { value: x, configurable: true })
    if (event.type === 'mouseup') stop()
  }
  const stop = () => {
    drag = null
    win.removeEventListener('mousemove', rewrite, true)
    win.removeEventListener('mouseup', rewrite, true)
  }
  const start = (event: MouseEvent) => {
    const factor = zoom()
    if (event.button !== 0 || Math.abs(factor - 1) < 0.001) return
    // prosemirror-tables marks the editor with `resize-cursor` exactly while a
    // column handle is active, which is when its mousedown starts a drag.
    const target = event.target instanceof Element ? event.target : null
    if (!target?.closest('.ProseMirror.resize-cursor')) return
    stop()
    drag = { startX: event.clientX, zoom: factor }
    win.addEventListener('mousemove', rewrite, true)
    win.addEventListener('mouseup', rewrite, true)
  }
  root.addEventListener('mousedown', start, true)
  return () => { root.removeEventListener('mousedown', start, true); stop() }
}
